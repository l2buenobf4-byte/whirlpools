//! Test-only helpers that build pinocchio memory-mapped views from Anchor state structs,
//! so the pinocchio port can be checked against the Anchor implementation.

use crate::pinocchio::errors::{UnifiedError, WhirlpoolErrorCode};
use crate::pinocchio::state::whirlpool::{
    MemoryMappedPosition, MemoryMappedTick, MemoryMappedWhirlpool,
};
use crate::state::{
    Position, PositionRewardInfo, Tick, Whirlpool, WhirlpoolRewardInfo, MAX_TICK_INDEX,
    MIN_TICK_INDEX, NUM_REWARDS,
};
use anchor_lang::prelude::Pubkey;
use anchor_lang::AccountSerialize;
use std::marker::PhantomData;

/// Owned account bytes viewed as a memory-mapped pinocchio struct.
pub struct MemoryMapped<T> {
    bytes: Vec<u8>,
    _marker: PhantomData<T>,
}

impl<T> MemoryMapped<T> {
    fn new(bytes: Vec<u8>) -> Self {
        assert!(
            bytes.len() >= core::mem::size_of::<T>(),
            "account bytes are shorter than the memory-mapped struct"
        );
        // All memory-mapped structs are built from byte arrays only.
        assert_eq!(core::mem::align_of::<T>(), 1);
        Self {
            bytes,
            _marker: PhantomData,
        }
    }

    pub fn get(&self) -> &T {
        unsafe { &*(self.bytes.as_ptr() as *const T) }
    }

    pub fn get_mut(&mut self) -> &mut T {
        unsafe { &mut *(self.bytes.as_mut_ptr() as *mut T) }
    }
}

fn serialize_account<A: AccountSerialize>(account: &A) -> Vec<u8> {
    let mut bytes = Vec::new();
    account.try_serialize(&mut bytes).unwrap();
    bytes
}

pub fn memory_mapped_whirlpool(whirlpool: &Whirlpool) -> MemoryMapped<MemoryMappedWhirlpool> {
    MemoryMapped::new(serialize_account(whirlpool))
}

pub fn memory_mapped_position(position: &Position) -> MemoryMapped<MemoryMappedPosition> {
    MemoryMapped::new(serialize_account(position))
}

pub fn memory_mapped_tick(tick: &Tick) -> MemoryMapped<MemoryMappedTick> {
    let mut bytes = Vec::with_capacity(crate::state::Tick::LEN);
    bytes.push(tick.initialized as u8);
    bytes.extend_from_slice(&{ tick.liquidity_net }.to_le_bytes());
    bytes.extend_from_slice(&{ tick.liquidity_gross }.to_le_bytes());
    bytes.extend_from_slice(&{ tick.fee_growth_outside_a }.to_le_bytes());
    bytes.extend_from_slice(&{ tick.fee_growth_outside_b }.to_le_bytes());
    let reward_growths_outside = tick.reward_growths_outside;
    for growth in reward_growths_outside {
        bytes.extend_from_slice(&growth.to_le_bytes());
    }
    MemoryMapped::new(bytes)
}

/// `UnifiedError` does not implement `Debug`, so `unwrap` is not available on pinocchio results.
pub fn expect_ok<T>(result: crate::pinocchio::Result<T>) -> T {
    match result {
        Ok(value) => value,
        Err(error) => panic!("unexpected error code {}", error_code(error)),
    }
}

pub fn expect_err_code<T>(result: crate::pinocchio::Result<T>) -> u64 {
    match result {
        Ok(_) => panic!("expected an error"),
        Err(error) => error_code(error),
    }
}

pub fn error_code(error: UnifiedError) -> u64 {
    error.into()
}

pub fn whirlpool_error_code(error: WhirlpoolErrorCode) -> u64 {
    error_code(error.into())
}

/// Small deterministic PRNG (xorshift64*) biased towards boundary values.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed.max(1))
    }

    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    pub fn bool(&mut self) -> bool {
        self.next_u64() & 1 == 1
    }

    pub fn below(&mut self, bound: u64) -> u64 {
        self.next_u64() % bound
    }

    pub fn u128(&mut self) -> u128 {
        match self.below(8) {
            0 => 0,
            1 => 1,
            2 => u128::MAX,
            3 => u128::MAX - self.below(1_000) as u128,
            4 => self.next_u64() as u128,
            5 => 1u128 << self.below(128),
            _ => ((self.next_u64() as u128) << 64) | self.next_u64() as u128,
        }
    }

    pub fn u64(&mut self) -> u64 {
        match self.below(6) {
            0 => 0,
            1 => u64::MAX,
            2 => u64::MAX - self.below(1_000),
            3 => self.below(1_000_000),
            _ => self.next_u64(),
        }
    }

    pub fn i128(&mut self) -> i128 {
        match self.below(6) {
            0 => 0,
            1 => i128::MAX,
            2 => i128::MIN,
            _ => self.u128() as i128,
        }
    }

    /// Liquidity amounts that real positions/ticks can hold (fits in i128).
    pub fn liquidity(&mut self) -> u128 {
        self.u128() & (i128::MAX as u128)
    }

    pub fn tick_index(&mut self) -> i32 {
        let span = (MAX_TICK_INDEX - MIN_TICK_INDEX + 1) as u64;
        MIN_TICK_INDEX + self.below(span) as i32
    }

    pub fn tick(&mut self) -> Tick {
        if self.below(4) == 0 {
            // uninitialized ticks are zeroed on chain
            return Tick::default();
        }
        Tick {
            initialized: true,
            liquidity_net: self.i128(),
            liquidity_gross: self.liquidity(),
            fee_growth_outside_a: self.u128(),
            fee_growth_outside_b: self.u128(),
            reward_growths_outside: [self.u128(), self.u128(), self.u128()],
        }
    }

    pub fn whirlpool(&mut self) -> Whirlpool {
        let mut reward_infos = [WhirlpoolRewardInfo::default(); NUM_REWARDS];
        for reward_info in reward_infos.iter_mut() {
            // On chain, an uninitialized reward never has emissions or growth.
            if self.below(3) != 0 {
                reward_info.mint = Pubkey::new_unique();
                reward_info.emissions_per_second_x64 = self.u128();
                reward_info.growth_global_x64 = self.u128();
            }
        }
        Whirlpool {
            tick_spacing: 64,
            liquidity: self.u128(),
            tick_current_index: self.tick_index(),
            fee_growth_global_a: self.u128(),
            fee_growth_global_b: self.u128(),
            reward_last_updated_timestamp: self.u64(),
            reward_infos,
            ..Default::default()
        }
    }

    pub fn position(&mut self) -> Position {
        let mut reward_infos = [PositionRewardInfo::default(); NUM_REWARDS];
        for reward_info in reward_infos.iter_mut() {
            reward_info.growth_inside_checkpoint = self.u128();
            reward_info.amount_owed = self.u64();
        }
        Position {
            liquidity: self.liquidity(),
            tick_lower_index: self.tick_index(),
            tick_upper_index: self.tick_index(),
            fee_growth_checkpoint_a: self.u128(),
            fee_owed_a: self.u64(),
            fee_growth_checkpoint_b: self.u128(),
            fee_owed_b: self.u64(),
            reward_infos,
            ..Default::default()
        }
    }
}

/// In-memory account in the layout the pinocchio entrypoint produces.
#[repr(C)]
pub struct RawAccount<const N: usize> {
    borrow_state: u8,
    is_signer: u8,
    is_writable: u8,
    executable: u8,
    resize_delta: i32,
    key: pinocchio::pubkey::Pubkey,
    owner: pinocchio::pubkey::Pubkey,
    lamports: u64,
    data_len: u64,
    data: [u8; N],
}

impl<const N: usize> RawAccount<N> {
    pub fn new(owner: pinocchio::pubkey::Pubkey, data: [u8; N]) -> Box<Self> {
        Box::new(Self {
            borrow_state: 0b_1111_1111,
            is_signer: 0,
            is_writable: 0,
            executable: 0,
            resize_delta: 0,
            key: [0u8; 32],
            owner,
            lamports: 0,
            data_len: N as u64,
            data,
        })
    }

    /// The returned AccountInfo must not outlive `self`.
    pub fn account_info(&mut self) -> pinocchio::account_info::AccountInfo {
        let mut slot = core::mem::MaybeUninit::<pinocchio::account_info::AccountInfo>::uninit();
        unsafe {
            (slot.as_mut_ptr() as *mut *mut Self).write(self as *mut Self);
            slot.assume_init()
        }
    }
}

/// SPL Token mint (no extensions, so no transfer fee).
pub fn raw_spl_token_mint() -> Box<RawAccount<82>> {
    let mut data = [0u8; 82];
    data[44] = 6; // decimals
    data[45] = 1; // is_initialized
    RawAccount::new(crate::pinocchio::constants::address::TOKEN_PROGRAM_ID, data)
}
