use crate::pinocchio::{
    constants::address::OPTIONAL_NON_ZERO_PUBKEY_NONE,
    cpi::{
        memo_build_memo::BuildMemo,
        token_transfer::Transfer,
        token_transfer_checked::{TransferChecked, TransferCheckedWithHook},
    },
    errors::{AnchorErrorCode, WhirlpoolErrorCode},
    state::{
        token::{
            extensions::{parse_token_extensions, TokenExtensions},
            MemoryMappedTokenAccount, MemoryMappedTokenMint,
        },
        whirlpool::MemoryMappedWhirlpool,
    },
    utils::account_load::{load_token_program_account, load_token_program_account_unchecked},
    Result,
};
use crate::util::{TransferFeeExcludedAmount, TransferFeeIncludedAmount};
use anchor_spl::token_2022::spl_token_2022::extension::transfer_fee::{
    TransferFee, MAX_FEE_BASIS_POINTS,
};
use pinocchio::sysvars::{clock::Clock, Sysvar};
use pinocchio::{
    account_info::AccountInfo,
    pubkey::{pubkey_eq, Pubkey},
};

pub fn pino_calculate_transfer_fee_excluded_amount(
    token_mint_info: &AccountInfo,
    transfer_fee_included_amount: u64,
) -> Result<TransferFeeExcludedAmount> {
    let token_mint =
        load_token_program_account_unchecked::<MemoryMappedTokenMint>(token_mint_info)?;
    let token_mint_extensions = parse_token_extensions(token_mint.extensions_tlv_data())?;
    if let Some(epoch_transfer_fee) = pino_get_epoch_transfer_fee(&token_mint_extensions)? {
        let transfer_fee = epoch_transfer_fee
            .calculate_fee(transfer_fee_included_amount)
            .unwrap();
        let transfer_fee_excluded_amount = transfer_fee_included_amount
            .checked_sub(transfer_fee)
            .unwrap();
        return Ok(TransferFeeExcludedAmount {
            amount: transfer_fee_excluded_amount,
            transfer_fee,
        });
    }

    Ok(TransferFeeExcludedAmount {
        amount: transfer_fee_included_amount,
        transfer_fee: 0,
    })
}

pub fn pino_calculate_transfer_fee_included_amount(
    token_mint_info: &AccountInfo,
    transfer_fee_excluded_amount: u64,
) -> Result<TransferFeeIncludedAmount> {
    if transfer_fee_excluded_amount == 0 {
        return Ok(TransferFeeIncludedAmount {
            amount: 0,
            transfer_fee: 0,
        });
    }

    // now transfer_fee_excluded_amount > 0

    // This mint account is stored as token_mint_a or token_mint_b of the whirlpool, so it must be valid Mint account.
    let token_mint =
        load_token_program_account_unchecked::<MemoryMappedTokenMint>(token_mint_info)?;
    let token_mint_extensions = parse_token_extensions(token_mint.extensions_tlv_data())?;
    if let Some(epoch_transfer_fee) = pino_get_epoch_transfer_fee(&token_mint_extensions)? {
        let transfer_fee: u64 =
            if u16::from(epoch_transfer_fee.transfer_fee_basis_points) == MAX_FEE_BASIS_POINTS {
                // edge-case: if transfer fee rate is 100%, current SPL implementation returns 0 as inverse fee.
                // https://github.com/solana-labs/solana-program-library/blob/fe1ac9a2c4e5d85962b78c3fc6aaf028461e9026/token/program-2022/src/extension/transfer_fee/mod.rs#L95

                // But even if transfer fee is 100%, we can use maximum_fee as transfer fee.
                // if transfer_fee_excluded_amount + maximum_fee > u64 max, the following checked_add should fail.
                u64::from(epoch_transfer_fee.maximum_fee)
            } else {
                epoch_transfer_fee
                    .calculate_inverse_fee(transfer_fee_excluded_amount)
                    .ok_or(WhirlpoolErrorCode::TransferFeeCalculationError)?
            };

        let transfer_fee_included_amount =
            transfer_fee_excluded_amount
                .checked_add(transfer_fee)
                .ok_or(WhirlpoolErrorCode::TransferFeeCalculationError)?;

        // verify transfer fee calculation for safety
        let transfer_fee_verification = epoch_transfer_fee
            .calculate_fee(transfer_fee_included_amount)
            .unwrap();
        if transfer_fee != transfer_fee_verification {
            // We believe this should never happen
            return Err(WhirlpoolErrorCode::TransferFeeCalculationError.into());
        }

        return Ok(TransferFeeIncludedAmount {
            amount: transfer_fee_included_amount,
            transfer_fee,
        });
    }

    Ok(TransferFeeIncludedAmount {
        amount: transfer_fee_excluded_amount,
        transfer_fee: 0,
    })
}

fn pino_get_epoch_transfer_fee(token_extensions: &TokenExtensions) -> Result<Option<TransferFee>> {
    match token_extensions.transfer_fee_config {
        None => Ok(None),
        Some(config) => {
            let epoch = Clock::get()?.epoch;

            if epoch >= config.newer_transfer_fee_epoch() {
                Ok(Some(TransferFee {
                    epoch: config.newer_transfer_fee_epoch().into(),
                    transfer_fee_basis_points: config
                        .newer_transfer_fee_transfer_fee_basis_points()
                        .into(),
                    maximum_fee: config.newer_transfer_fee_maximum_fee().into(),
                }))
            } else {
                Ok(Some(TransferFee {
                    epoch: config.older_transfer_fee_epoch().into(),
                    transfer_fee_basis_points: config
                        .older_transfer_fee_transfer_fee_basis_points()
                        .into(),
                    maximum_fee: config.older_transfer_fee_maximum_fee().into(),
                }))
            }
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn pino_transfer_from_owner_to_vault_v2(
    authority_info: &AccountInfo,
    token_mint_info: &AccountInfo,
    token_owner_account_info: &AccountInfo,
    token_vault_info: &AccountInfo,
    token_program_info: &AccountInfo,
    memo_program_info: &AccountInfo,
    transfer_hook_account_infos: &Option<Vec<&AccountInfo>>,
    amount: u64,
) -> Result<()> {
    // This mint account is stored as token_mint_a or token_mint_b of the whirlpool, so it must be valid Mint account.
    let token_mint =
        load_token_program_account_unchecked::<MemoryMappedTokenMint>(token_mint_info)?;
    let decimals = token_mint.decimals();
    let token_mint_extensions = parse_token_extensions(token_mint.extensions_tlv_data())?;

    // TransferFee extension
    let epoch_transfer_fee = pino_get_epoch_transfer_fee(&token_mint_extensions)?;
    if let Some(epoch_transfer_fee) = &epoch_transfer_fee {
        // log applied transfer fee
        // - Not must, but important for ease of investigation and replay when problems occur
        // - Use Memo because logs risk being truncated
        let transfer_fee_memo = format!(
            "TFe: {}, {}",
            u16::from(epoch_transfer_fee.transfer_fee_basis_points),
            u64::from(epoch_transfer_fee.maximum_fee),
        );
        BuildMemo {
            program: memo_program_info,
            memo: transfer_fee_memo.as_bytes(),
        }
        .invoke_signed(&[])?;
    }
    let transfer_fee = match &epoch_transfer_fee {
        Some(epoch_transfer_fee) => epoch_transfer_fee
            .calculate_fee(amount)
            .ok_or(WhirlpoolErrorCode::TransferFeeCalculationError)?,
        None => 0,
    };
    let vault_before = pino_vault_snapshot(token_vault_info)?;

    // MemoTransfer extension
    // The vault doesn't have MemoTransfer extension, so we don't need to use memo_program here

    // TransferHook extension
    if pino_is_transfer_hook_enabled(&token_mint_extensions) {
        let transfer_hook_accounts = transfer_hook_account_infos
            .as_deref()
            .ok_or(WhirlpoolErrorCode::NoExtraAccountsForTransferHook)?;

        TransferCheckedWithHook {
            program: token_program_info,
            from: token_owner_account_info,
            mint: token_mint_info,
            to: token_vault_info,
            authority: authority_info,
            transfer_hook_accounts,
            amount,
            decimals,
        }
        .invoke_signed(&[])?;
    } else {
        TransferChecked {
            program: token_program_info,
            from: token_owner_account_info,
            mint: token_mint_info,
            to: token_vault_info,
            authority: authority_info,
            amount,
            decimals,
        }
        .invoke_signed(&[])?;
    }

    // The vault must have received exactly the amount minus the transfer fee, and its owner,
    // delegate and close authority must be unchanged, whatever a transfer hook or the token
    // program did during the transfer.
    let vault_amount_received = amount
        .checked_sub(transfer_fee)
        .ok_or(WhirlpoolErrorCode::TransferFeeCalculationError)?;
    pino_verify_vault_after_transfer(
        token_vault_info,
        &vault_before,
        VaultChange::Increase(vault_amount_received),
    )
}

#[allow(clippy::too_many_arguments)]
pub fn pino_transfer_from_vault_to_owner_v2(
    whirlpool: &MemoryMappedWhirlpool,
    whirlpool_info: &AccountInfo,
    token_mint_info: &AccountInfo,
    token_vault_info: &AccountInfo,
    token_owner_account_info: &AccountInfo,
    token_program_info: &AccountInfo,
    memo_program_info: &AccountInfo,
    transfer_hook_account_infos: &Option<Vec<&AccountInfo>>,
    amount: u64,
    memo: &[u8],
) -> Result<()> {
    // This mint account is stored as token_mint_a or token_mint_b of the whirlpool, so it must be valid Mint account.
    let token_mint =
        load_token_program_account_unchecked::<MemoryMappedTokenMint>(token_mint_info)?;
    let decimals = token_mint.decimals();
    let token_mint_extensions = parse_token_extensions(token_mint.extensions_tlv_data())?;

    // TransferFee extension
    if let Some(epoch_transfer_fee) = pino_get_epoch_transfer_fee(&token_mint_extensions)? {
        // log applied transfer fee
        // - Not must, but important for ease of investigation and replay when problems occur
        // - Use Memo because logs risk being truncated
        let transfer_fee_memo = format!(
            "TFe: {}, {}",
            u16::from(epoch_transfer_fee.transfer_fee_basis_points),
            u64::from(epoch_transfer_fee.maximum_fee),
        );
        BuildMemo {
            program: memo_program_info,
            memo: transfer_fee_memo.as_bytes(),
        }
        .invoke_signed(&[])?;
    }

    let vault_before = pino_vault_snapshot(token_vault_info)?;

    // MemoTransfer extension
    let token_owner_account =
        load_token_program_account::<MemoryMappedTokenAccount>(token_owner_account_info)?;
    let token_owner_account_extensions =
        parse_token_extensions(token_owner_account.extensions_tlv_data())?;
    if pino_is_transfer_memo_required(&token_owner_account_extensions) {
        BuildMemo {
            program: memo_program_info,
            memo,
        }
        .invoke_signed(&[])?;
    }
    drop(token_owner_account);

    // TransferHook extension
    if pino_is_transfer_hook_enabled(&token_mint_extensions) {
        let transfer_hook_accounts = transfer_hook_account_infos
            .as_deref()
            .ok_or(WhirlpoolErrorCode::NoExtraAccountsForTransferHook)?;

        TransferCheckedWithHook {
            program: token_program_info,
            from: token_vault_info,
            mint: token_mint_info,
            to: token_owner_account_info,
            authority: whirlpool_info,
            transfer_hook_accounts,
            amount,
            decimals,
        }
        .invoke_signed(&[whirlpool.seeds().as_ref().into()])?;
    } else {
        TransferChecked {
            program: token_program_info,
            from: token_vault_info,
            mint: token_mint_info,
            to: token_owner_account_info,
            authority: whirlpool_info,
            amount,
            decimals,
        }
        .invoke_signed(&[whirlpool.seeds().as_ref().into()])?;
    }

    // The vault must have sent exactly `amount`, and its owner, delegate and close authority
    // must be unchanged, whatever a transfer hook or the token program did during the transfer.
    pino_verify_vault_after_transfer(
        token_vault_info,
        &vault_before,
        VaultChange::Decrease(amount),
    )
}

fn pino_is_transfer_hook_enabled(token_extensions: &TokenExtensions) -> bool {
    match token_extensions.transfer_hook {
        None => false,
        Some(hook) => !pubkey_eq(hook.program_id(), &OPTIONAL_NON_ZERO_PUBKEY_NONE),
    }
}

fn pino_is_transfer_memo_required(token_extensions: &TokenExtensions) -> bool {
    match token_extensions.memo_transfer {
        None => false,
        Some(memo_transfer) => memo_transfer.is_memo_required(),
    }
}

pub fn pino_transfer_from_owner_to_vault(
    authority_info: &AccountInfo,
    token_owner_account_info: &AccountInfo,
    token_vault_info: &AccountInfo,
    token_program_info: &AccountInfo,
    amount: u64,
) -> Result<()> {
    Transfer {
        program: token_program_info,
        from: token_owner_account_info,
        to: token_vault_info,
        authority: authority_info,
        amount,
    }
    .invoke_signed(&[])?;
    Ok(())
}

pub fn pino_transfer_from_vault_to_owner(
    whirlpool: &MemoryMappedWhirlpool,
    whirlpool_info: &AccountInfo,
    token_vault_info: &AccountInfo,
    token_owner_account_info: &AccountInfo,
    token_program_info: &AccountInfo,
    amount: u64,
) -> Result<()> {
    Transfer {
        program: token_program_info,
        from: token_vault_info,
        to: token_owner_account_info,
        authority: whirlpool_info,
        amount,
    }
    .invoke_signed(&[whirlpool.seeds().as_ref().into()])?;
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum VaultChange {
    Increase(u64),
    Decrease(u64),
}

/// The parts of a vault token account that a transfer must never change, plus its balance.
#[derive(Debug, PartialEq, Eq)]
struct VaultSnapshot {
    amount: u64,
    owner: Pubkey,
    delegate: Option<Pubkey>,
    close_authority: Option<Pubkey>,
}

fn pino_vault_snapshot(token_vault_info: &AccountInfo) -> Result<VaultSnapshot> {
    let vault = load_token_program_account::<MemoryMappedTokenAccount>(token_vault_info)?;
    Ok(VaultSnapshot {
        amount: vault.amount(),
        owner: *vault.owner(),
        delegate: vault.delegate().copied(),
        close_authority: vault.close_authority().copied(),
    })
}

/// Fail unless the vault balance moved by exactly the expected amount and nothing that
/// controls the vault (owner, delegate, close authority) changed during the transfer.
fn pino_verify_vault_after_transfer(
    token_vault_info: &AccountInfo,
    before: &VaultSnapshot,
    change: VaultChange,
) -> Result<()> {
    let after = pino_vault_snapshot(token_vault_info)?;
    let expected = VaultSnapshot {
        amount: pino_expected_vault_amount(before.amount, change)
            .ok_or(AnchorErrorCode::ConstraintRaw)?,
        owner: before.owner,
        delegate: before.delegate,
        close_authority: before.close_authority,
    };
    if after != expected {
        return Err(AnchorErrorCode::ConstraintRaw.into());
    }
    Ok(())
}

fn pino_expected_vault_amount(amount_before: u64, change: VaultChange) -> Option<u64> {
    match change {
        VaultChange::Increase(amount) => amount_before.checked_add(amount),
        VaultChange::Decrease(amount) => amount_before.checked_sub(amount),
    }
}

#[cfg(test)]
mod vault_amount_tests {
    use super::*;

    #[test]
    fn expected_vault_amount_is_exact() {
        assert_eq!(
            pino_expected_vault_amount(100, VaultChange::Increase(5)),
            Some(105)
        );
        assert_eq!(
            pino_expected_vault_amount(100, VaultChange::Decrease(5)),
            Some(95)
        );
        assert_eq!(
            pino_expected_vault_amount(100, VaultChange::Increase(0)),
            Some(100)
        );
        assert_eq!(
            pino_expected_vault_amount(5, VaultChange::Decrease(6)),
            None
        );
        assert_eq!(
            pino_expected_vault_amount(u64::MAX, VaultChange::Increase(1)),
            None
        );
    }

    fn vault_data(
        amount: u64,
        owner: [u8; 32],
        delegate: Option<[u8; 32]>,
        close: Option<[u8; 32]>,
    ) -> [u8; 165] {
        let mut data = [0u8; 165];
        data[32..64].copy_from_slice(&owner);
        data[64..72].copy_from_slice(&amount.to_le_bytes());
        if let Some(delegate) = delegate {
            data[72] = 1;
            data[76..108].copy_from_slice(&delegate);
        }
        data[108] = 1; // initialized
        if let Some(close) = close {
            data[129] = 1;
            data[133..165].copy_from_slice(&close);
        }
        data
    }

    fn check(before: [u8; 165], after: [u8; 165], change: VaultChange) -> bool {
        use crate::pinocchio::{constants::address::TOKEN_PROGRAM_ID, test_utils::RawAccount};
        let mut before_account = RawAccount::new(TOKEN_PROGRAM_ID, before);
        let snapshot = pino_vault_snapshot(&before_account.account_info())
            .ok()
            .unwrap();
        let mut after_account = RawAccount::new(TOKEN_PROGRAM_ID, after);
        pino_verify_vault_after_transfer(&after_account.account_info(), &snapshot, change).is_ok()
    }

    #[test]
    fn vault_check_reads_the_live_balance() {
        let pool = [1u8; 32];
        let before = vault_data(1_100, pool, None, None);
        assert!(check(
            before,
            vault_data(1_000, pool, None, None),
            VaultChange::Decrease(100)
        ));
        // a hook or token program that moved one extra unit out must be caught
        assert!(!check(
            before,
            vault_data(999, pool, None, None),
            VaultChange::Decrease(100)
        ));
        // a deposit that arrived short must be caught
        assert!(!check(
            before,
            vault_data(1_199, pool, None, None),
            VaultChange::Increase(100)
        ));
        assert!(check(
            before,
            vault_data(1_200, pool, None, None),
            VaultChange::Increase(100)
        ));
    }

    #[test]
    fn vault_check_catches_authority_changes_with_correct_balance() {
        let (pool, attacker) = ([1u8; 32], [9u8; 32]);
        let before = vault_data(1_100, pool, None, None);
        let ok_after = |owner, delegate, close| vault_data(1_000, owner, delegate, close);
        assert!(check(
            before,
            ok_after(pool, None, None),
            VaultChange::Decrease(100)
        ));
        // owner changed (SetAuthority AccountOwner)
        assert!(!check(
            before,
            ok_after(attacker, None, None),
            VaultChange::Decrease(100)
        ));
        // delegate added (Approve)
        assert!(!check(
            before,
            ok_after(pool, Some(attacker), None),
            VaultChange::Decrease(100)
        ));
        // close authority set (SetAuthority CloseAccount)
        assert!(!check(
            before,
            ok_after(pool, None, Some(attacker)),
            VaultChange::Decrease(100)
        ));
    }
}
