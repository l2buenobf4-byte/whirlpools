use crate::pinocchio::errors::WhirlpoolErrorCode;
use crate::pinocchio::Result;
use crate::util::{AccountsType, RemainingAccountsInfo, MAX_SUPPLEMENTAL_TICK_ARRAYS_LEN};
use pinocchio::account_info::AccountInfo;
use pinocchio::pubkey::{pubkey_eq, Pubkey};

#[derive(Default)]
pub struct PinoParsedRemainingAccounts<'a> {
    pub transfer_hook_a: Option<Vec<&'a AccountInfo>>,
    pub transfer_hook_b: Option<Vec<&'a AccountInfo>>,
    pub transfer_hook_reward: Option<Vec<&'a AccountInfo>>,
    pub transfer_hook_input: Option<Vec<&'a AccountInfo>>,
    pub transfer_hook_intermediate: Option<Vec<&'a AccountInfo>>,
    pub transfer_hook_output: Option<Vec<&'a AccountInfo>>,
    pub supplemental_tick_arrays: Option<Vec<&'a AccountInfo>>,
    pub supplemental_tick_arrays_one: Option<Vec<&'a AccountInfo>>,
    pub supplemental_tick_arrays_two: Option<Vec<&'a AccountInfo>>,
    pub transfer_hook_deposit_a: Option<Vec<&'a AccountInfo>>,
    pub transfer_hook_deposit_b: Option<Vec<&'a AccountInfo>>,
    pub transfer_hook_withdrawal_a: Option<Vec<&'a AccountInfo>>,
    pub transfer_hook_withdrawal_b: Option<Vec<&'a AccountInfo>>,
}

pub fn pino_parse_remaining_accounts<'a>(
    remaining_accounts: &'a [AccountInfo],
    remaining_accounts_info: &Option<RemainingAccountsInfo>,
    valid_accounts_type_list: &[AccountsType],
) -> Result<PinoParsedRemainingAccounts<'a>> {
    let mut remaining_accounts_iter = remaining_accounts.iter();
    let mut parsed_remaining_accounts = PinoParsedRemainingAccounts::default();

    if remaining_accounts_info.is_none() {
        return Ok(parsed_remaining_accounts);
    }

    let remaining_accounts_info = remaining_accounts_info.as_ref().unwrap();

    for slice in remaining_accounts_info.slices.iter() {
        if !valid_accounts_type_list.contains(&slice.accounts_type) {
            return Err(WhirlpoolErrorCode::RemainingAccountsInvalidSlice.into());
        }
        if slice.length == 0 {
            continue;
        }

        let mut accounts: Vec<&AccountInfo> = Vec::with_capacity(slice.length as usize);
        for _ in 0..slice.length {
            if let Some(account) = remaining_accounts_iter.next() {
                accounts.push(account);
            } else {
                return Err(WhirlpoolErrorCode::RemainingAccountsInsufficient.into());
            }
        }

        match slice.accounts_type {
            AccountsType::TransferHookA => {
                if parsed_remaining_accounts.transfer_hook_a.is_some() {
                    return Err(WhirlpoolErrorCode::RemainingAccountsDuplicatedAccountsType.into());
                }
                parsed_remaining_accounts.transfer_hook_a = Some(accounts);
            }
            AccountsType::TransferHookB => {
                if parsed_remaining_accounts.transfer_hook_b.is_some() {
                    return Err(WhirlpoolErrorCode::RemainingAccountsDuplicatedAccountsType.into());
                }
                parsed_remaining_accounts.transfer_hook_b = Some(accounts);
            }
            AccountsType::TransferHookReward => {
                if parsed_remaining_accounts.transfer_hook_reward.is_some() {
                    return Err(WhirlpoolErrorCode::RemainingAccountsDuplicatedAccountsType.into());
                }
                parsed_remaining_accounts.transfer_hook_reward = Some(accounts);
            }
            AccountsType::TransferHookInput => {
                if parsed_remaining_accounts.transfer_hook_input.is_some() {
                    return Err(WhirlpoolErrorCode::RemainingAccountsDuplicatedAccountsType.into());
                }
                parsed_remaining_accounts.transfer_hook_input = Some(accounts);
            }
            AccountsType::TransferHookIntermediate => {
                if parsed_remaining_accounts
                    .transfer_hook_intermediate
                    .is_some()
                {
                    return Err(WhirlpoolErrorCode::RemainingAccountsDuplicatedAccountsType.into());
                }
                parsed_remaining_accounts.transfer_hook_intermediate = Some(accounts);
            }
            AccountsType::TransferHookOutput => {
                if parsed_remaining_accounts.transfer_hook_output.is_some() {
                    return Err(WhirlpoolErrorCode::RemainingAccountsDuplicatedAccountsType.into());
                }
                parsed_remaining_accounts.transfer_hook_output = Some(accounts);
            }
            AccountsType::SupplementalTickArrays => {
                if accounts.len() > MAX_SUPPLEMENTAL_TICK_ARRAYS_LEN {
                    return Err(WhirlpoolErrorCode::TooManySupplementalTickArrays.into());
                }

                if parsed_remaining_accounts.supplemental_tick_arrays.is_some() {
                    return Err(WhirlpoolErrorCode::RemainingAccountsDuplicatedAccountsType.into());
                }
                parsed_remaining_accounts.supplemental_tick_arrays = Some(accounts);
            }
            AccountsType::SupplementalTickArraysOne => {
                if accounts.len() > MAX_SUPPLEMENTAL_TICK_ARRAYS_LEN {
                    return Err(WhirlpoolErrorCode::TooManySupplementalTickArrays.into());
                }

                if parsed_remaining_accounts
                    .supplemental_tick_arrays_one
                    .is_some()
                {
                    return Err(WhirlpoolErrorCode::RemainingAccountsDuplicatedAccountsType.into());
                }
                parsed_remaining_accounts.supplemental_tick_arrays_one = Some(accounts);
            }
            AccountsType::SupplementalTickArraysTwo => {
                if accounts.len() > MAX_SUPPLEMENTAL_TICK_ARRAYS_LEN {
                    return Err(WhirlpoolErrorCode::TooManySupplementalTickArrays.into());
                }

                if parsed_remaining_accounts
                    .supplemental_tick_arrays_two
                    .is_some()
                {
                    return Err(WhirlpoolErrorCode::RemainingAccountsDuplicatedAccountsType.into());
                }
                parsed_remaining_accounts.supplemental_tick_arrays_two = Some(accounts);
            }
            AccountsType::TransferHookDepositA => {
                if parsed_remaining_accounts.transfer_hook_deposit_a.is_some() {
                    return Err(WhirlpoolErrorCode::RemainingAccountsDuplicatedAccountsType.into());
                }
                parsed_remaining_accounts.transfer_hook_deposit_a = Some(accounts);
            }
            AccountsType::TransferHookDepositB => {
                if parsed_remaining_accounts.transfer_hook_deposit_b.is_some() {
                    return Err(WhirlpoolErrorCode::RemainingAccountsDuplicatedAccountsType.into());
                }
                parsed_remaining_accounts.transfer_hook_deposit_b = Some(accounts);
            }
            AccountsType::TransferHookWithdrawalA => {
                if parsed_remaining_accounts
                    .transfer_hook_withdrawal_a
                    .is_some()
                {
                    return Err(WhirlpoolErrorCode::RemainingAccountsDuplicatedAccountsType.into());
                }
                parsed_remaining_accounts.transfer_hook_withdrawal_a = Some(accounts);
            }
            AccountsType::TransferHookWithdrawalB => {
                if parsed_remaining_accounts
                    .transfer_hook_withdrawal_b
                    .is_some()
                {
                    return Err(WhirlpoolErrorCode::RemainingAccountsDuplicatedAccountsType.into());
                }
                parsed_remaining_accounts.transfer_hook_withdrawal_b = Some(accounts);
            }
        }
    }

    Ok(parsed_remaining_accounts)
}

/// Reject transfer-hook account lists that include any protected account (the pool, its
/// vaults, the position).
///
/// A hook has no legitimate need for these: the transfer's own source, destination and
/// authority are already passed to it by Token-2022. Keeping them out means a hook can't be
/// handed access to them by Whirlpool, whatever Token-2022 later does with account privileges
/// (see `security/reposition_fee_register.json`, A14).
pub fn pino_reject_protected_hook_accounts(
    remaining_accounts: &PinoParsedRemainingAccounts,
    protected: &[&Pubkey],
) -> Result<()> {
    let hook_account_lists = [
        &remaining_accounts.transfer_hook_a,
        &remaining_accounts.transfer_hook_b,
        &remaining_accounts.transfer_hook_reward,
        &remaining_accounts.transfer_hook_input,
        &remaining_accounts.transfer_hook_intermediate,
        &remaining_accounts.transfer_hook_output,
        &remaining_accounts.transfer_hook_deposit_a,
        &remaining_accounts.transfer_hook_deposit_b,
        &remaining_accounts.transfer_hook_withdrawal_a,
        &remaining_accounts.transfer_hook_withdrawal_b,
    ];
    for accounts in hook_account_lists.into_iter().flatten() {
        for account in accounts {
            if protected.iter().any(|key| pubkey_eq(account.key(), key)) {
                return Err(WhirlpoolErrorCode::RemainingAccountsInvalidSlice.into());
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod protected_hook_accounts_tests {
    use super::*;
    use crate::pinocchio::test_utils::{expect_err_code, whirlpool_error_code, RawAccount};

    #[test]
    fn rejects_any_protected_account_in_any_hook_list() {
        let (pool, vault_a, vault_b, position) = ([1u8; 32], [2u8; 32], [3u8; 32], [4u8; 32]);
        let protected = [&pool, &vault_a, &vault_b, &position];
        let mut counter = RawAccount::new([9u8; 32], [0u8; 8]).with_key([7u8; 32]);
        let counter_info = counter.account_info();

        for key in [pool, vault_a, vault_b, position] {
            let mut sneaky = RawAccount::new([9u8; 32], [0u8; 8]).with_key(key);
            let sneaky_info = sneaky.account_info();
            for slot in 0..10 {
                let mut parsed = PinoParsedRemainingAccounts::default();
                let list = Some(vec![&counter_info, &sneaky_info]);
                match slot {
                    0 => parsed.transfer_hook_a = list,
                    1 => parsed.transfer_hook_b = list,
                    2 => parsed.transfer_hook_reward = list,
                    3 => parsed.transfer_hook_input = list,
                    4 => parsed.transfer_hook_intermediate = list,
                    5 => parsed.transfer_hook_output = list,
                    6 => parsed.transfer_hook_deposit_a = list,
                    7 => parsed.transfer_hook_deposit_b = list,
                    8 => parsed.transfer_hook_withdrawal_a = list,
                    _ => parsed.transfer_hook_withdrawal_b = list,
                }
                assert_eq!(
                    expect_err_code(pino_reject_protected_hook_accounts(&parsed, &protected)),
                    whirlpool_error_code(WhirlpoolErrorCode::RemainingAccountsInvalidSlice)
                );
            }
        }
    }

    #[test]
    fn allows_hook_lists_without_protected_accounts() {
        let protected = [&[1u8; 32], &[2u8; 32], &[3u8; 32], &[4u8; 32]];
        let mut counter = RawAccount::new([9u8; 32], [0u8; 8]).with_key([7u8; 32]);
        let counter_info = counter.account_info();
        let parsed = PinoParsedRemainingAccounts {
            transfer_hook_deposit_a: Some(vec![&counter_info]),
            transfer_hook_withdrawal_b: Some(vec![&counter_info]),
            ..Default::default()
        };
        assert!(pino_reject_protected_hook_accounts(&parsed, &protected).is_ok());
        assert!(pino_reject_protected_hook_accounts(
            &PinoParsedRemainingAccounts::default(),
            &protected
        )
        .is_ok());
    }
}
