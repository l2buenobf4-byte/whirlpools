use super::super::{BytesI32, BytesU128, BytesU64, Pubkey};
use super::MemoryMappedWhirlpool;
use crate::pinocchio::state::whirlpool::NUM_REWARDS;
use crate::{
    errors::ErrorCode,
    math::FULL_RANGE_ONLY_TICK_SPACING_THRESHOLD,
    pinocchio::{state::WhirlpoolProgramAccount, Result},
    state::Tick,
};

#[repr(C)]
pub struct MemoryMappedPositionRewardInfo {
    growth_inside_checkpoint: BytesU128,
    amount_owed: BytesU64,
}

impl MemoryMappedPositionRewardInfo {
    #[inline(always)]
    pub fn growth_inside_checkpoint(&self) -> u128 {
        u128::from_le_bytes(self.growth_inside_checkpoint)
    }

    #[inline(always)]
    pub fn amount_owed(&self) -> u64 {
        u64::from_le_bytes(self.amount_owed)
    }
}

#[repr(C)]
pub struct MemoryMappedPosition {
    discriminator: [u8; 8],

    whirlpool: Pubkey,
    position_mint: Pubkey,
    liquidity: BytesU128,
    tick_lower_index: BytesI32,
    tick_upper_index: BytesI32,
    fee_growth_checkpoint_a: BytesU128,
    fee_owed_a: BytesU64,
    fee_growth_checkpoint_b: BytesU128,
    fee_owed_b: BytesU64,
    reward_infos: [MemoryMappedPositionRewardInfo; crate::state::NUM_REWARDS],
}

impl WhirlpoolProgramAccount for MemoryMappedPosition {
    const DISCRIMINATOR: [u8; 8] = [0xaa, 0xbc, 0x8f, 0xe4, 0x7a, 0x40, 0xf7, 0xd0];
}

impl MemoryMappedPosition {
    #[inline(always)]
    pub fn whirlpool(&self) -> &Pubkey {
        &self.whirlpool
    }

    #[inline(always)]
    pub fn position_mint(&self) -> &Pubkey {
        &self.position_mint
    }

    #[inline(always)]
    pub fn liquidity(&self) -> u128 {
        u128::from_le_bytes(self.liquidity)
    }

    #[inline(always)]
    pub fn tick_lower_index(&self) -> i32 {
        i32::from_le_bytes(self.tick_lower_index)
    }

    #[inline(always)]
    pub fn tick_upper_index(&self) -> i32 {
        i32::from_le_bytes(self.tick_upper_index)
    }

    #[inline(always)]
    pub fn fee_growth_checkpoint_a(&self) -> u128 {
        u128::from_le_bytes(self.fee_growth_checkpoint_a)
    }

    #[inline(always)]
    pub fn fee_owed_a(&self) -> u64 {
        u64::from_le_bytes(self.fee_owed_a)
    }

    #[inline(always)]
    pub fn fee_growth_checkpoint_b(&self) -> u128 {
        u128::from_le_bytes(self.fee_growth_checkpoint_b)
    }

    #[inline(always)]
    pub fn fee_owed_b(&self) -> u64 {
        u64::from_le_bytes(self.fee_owed_b)
    }

    #[inline(always)]
    pub fn reward_infos(&self) -> &[MemoryMappedPositionRewardInfo; crate::state::NUM_REWARDS] {
        &self.reward_infos
    }

    pub fn reset_position_range(
        &mut self,
        whirlpool: &MemoryMappedWhirlpool,
        new_tick_lower_index: i32,
        new_tick_upper_index: i32,
        keep_owed: bool,
    ) -> Result<()> {
        if !self.is_position_empty(keep_owed) {
            return Err(ErrorCode::ClosePositionNotEmpty.into());
        }

        if new_tick_lower_index == self.tick_lower_index()
            && new_tick_upper_index == self.tick_upper_index()
        {
            return Err(ErrorCode::SameTickRangeNotAllowed.into());
        }

        validate_tick_range_for_whirlpool(whirlpool, new_tick_lower_index, new_tick_upper_index)?;

        self.set_tick_lower_index(new_tick_lower_index);
        self.set_tick_upper_index(new_tick_upper_index);
        self.set_fee_growth_checkpoint_a(0);
        self.set_fee_growth_checkpoint_b(0);
        self.reset_reward_growth_checkpoints();

        Ok(())
    }

    pub fn update(&mut self, update: &crate::state::PositionUpdate) {
        self.set_liquidity(update.liquidity);
        self.set_fee_growth_checkpoint_a(update.fee_growth_checkpoint_a);
        self.set_fee_growth_checkpoint_b(update.fee_growth_checkpoint_b);
        self.set_fee_owed_a(update.fee_owed_a);
        self.set_fee_owed_b(update.fee_owed_b);
        self.set_reward_infos(&update.reward_infos);
    }

    fn is_position_empty(&self, keep_owed: bool) -> bool {
        if keep_owed {
            return self.liquidity() == 0;
        }

        let fees_not_owed = self.fee_owed_a() == 0 && self.fee_owed_b() == 0;
        let mut rewards_not_owed = true;
        for i in 0..NUM_REWARDS {
            rewards_not_owed = rewards_not_owed && self.reward_infos()[i].amount_owed() == 0
        }
        self.liquidity() == 0 && fees_not_owed && rewards_not_owed
    }

    fn set_liquidity(&mut self, liquidity: u128) {
        self.liquidity = liquidity.to_le_bytes();
    }

    fn set_tick_lower_index(&mut self, tick_lower_index: i32) {
        self.tick_lower_index = tick_lower_index.to_le_bytes();
    }

    fn set_tick_upper_index(&mut self, tick_upper_index: i32) {
        self.tick_upper_index = tick_upper_index.to_le_bytes();
    }

    fn set_fee_growth_checkpoint_a(&mut self, fee_growth_checkpoint_a: u128) {
        self.fee_growth_checkpoint_a = fee_growth_checkpoint_a.to_le_bytes();
    }

    fn set_fee_growth_checkpoint_b(&mut self, fee_growth_checkpoint_b: u128) {
        self.fee_growth_checkpoint_b = fee_growth_checkpoint_b.to_le_bytes();
    }

    fn set_fee_owed_a(&mut self, fee_owed_a: u64) {
        self.fee_owed_a = fee_owed_a.to_le_bytes();
    }

    fn set_fee_owed_b(&mut self, fee_owed_b: u64) {
        self.fee_owed_b = fee_owed_b.to_le_bytes();
    }

    fn set_reward_infos(
        &mut self,
        reward_infos: &[crate::state::PositionRewardInfo; crate::state::NUM_REWARDS],
    ) {
        self.reward_infos[0].amount_owed = reward_infos[0].amount_owed.to_le_bytes();
        self.reward_infos[0].growth_inside_checkpoint =
            reward_infos[0].growth_inside_checkpoint.to_le_bytes();
        self.reward_infos[1].amount_owed = reward_infos[1].amount_owed.to_le_bytes();
        self.reward_infos[1].growth_inside_checkpoint =
            reward_infos[1].growth_inside_checkpoint.to_le_bytes();
        self.reward_infos[2].amount_owed = reward_infos[2].amount_owed.to_le_bytes();
        self.reward_infos[2].growth_inside_checkpoint =
            reward_infos[2].growth_inside_checkpoint.to_le_bytes();
    }

    fn reset_reward_growth_checkpoints(&mut self) {
        for reward_info in &mut self.reward_infos {
            reward_info.growth_inside_checkpoint = 0u128.to_le_bytes();
        }
    }
}

fn validate_tick_range_for_whirlpool(
    whirlpool: &MemoryMappedWhirlpool,
    tick_lower_index: i32,
    tick_upper_index: i32,
) -> Result<()> {
    let tick_spacing = whirlpool.tick_spacing();

    if !Tick::check_is_usable_tick(tick_lower_index, tick_spacing)
        || !Tick::check_is_usable_tick(tick_upper_index, tick_spacing)
        || tick_lower_index >= tick_upper_index
    {
        return Err(ErrorCode::InvalidTickIndex.into());
    }

    if tick_spacing >= FULL_RANGE_ONLY_TICK_SPACING_THRESHOLD {
        let (full_range_lower_index, full_range_upper_index) =
            Tick::full_range_indexes(tick_spacing);
        if tick_lower_index != full_range_lower_index || tick_upper_index != full_range_upper_index
        {
            return Err(ErrorCode::FullRangeOnlyPool.into());
        }
    }

    Ok(())
}

#[cfg(test)]
mod reset_position_range_tests {
    use super::*;
    use crate::pinocchio::test_utils::*;
    use crate::state::{MAX_TICK_INDEX, MIN_TICK_INDEX};

    fn whirlpool(tick_spacing: u16) -> MemoryMapped<MemoryMappedWhirlpool> {
        memory_mapped_whirlpool(&crate::state::Whirlpool {
            tick_spacing,
            ..Default::default()
        })
    }

    fn position(
        liquidity: u128,
        fee_owed_a: u64,
        reward_owed: u64,
    ) -> MemoryMapped<MemoryMappedPosition> {
        let mut position = crate::state::Position {
            liquidity,
            tick_lower_index: -64,
            tick_upper_index: 64,
            fee_owed_a,
            ..Default::default()
        };
        position.reward_infos[2].amount_owed = reward_owed;
        memory_mapped_position(&position)
    }

    fn reset_error(
        position: &mut MemoryMapped<MemoryMappedPosition>,
        whirlpool: &MemoryMapped<MemoryMappedWhirlpool>,
        lower: i32,
        upper: i32,
        keep_owed: bool,
    ) -> u64 {
        let result =
            position
                .get_mut()
                .reset_position_range(whirlpool.get(), lower, upper, keep_owed);
        expect_err_code(result)
    }

    #[test]
    fn keep_owed_only_requires_zero_liquidity() {
        let whirlpool = whirlpool(64);
        let mut owed = position(0, 5, 7);
        expect_ok(
            owed.get_mut()
                .reset_position_range(whirlpool.get(), 0, 128, true),
        );
        assert_eq!(owed.get().fee_owed_a(), 5);
        assert_eq!(owed.get().reward_infos()[2].amount_owed(), 7);

        let mut with_liquidity = position(1, 0, 0);
        assert_eq!(
            reset_error(&mut with_liquidity, &whirlpool, 0, 128, true),
            whirlpool_error_code(ErrorCode::ClosePositionNotEmpty)
        );
    }

    #[test]
    fn without_keep_owed_owed_amounts_block_reset() {
        let whirlpool = whirlpool(64);
        for mut p in [position(0, 1, 0), position(0, 0, 1), position(1, 0, 0)] {
            assert_eq!(
                reset_error(&mut p, &whirlpool, 0, 128, false),
                whirlpool_error_code(ErrorCode::ClosePositionNotEmpty)
            );
        }
        expect_ok(
            position(0, 0, 0)
                .get_mut()
                .reset_position_range(whirlpool.get(), 0, 128, false),
        );
    }

    #[test]
    fn rejects_same_and_invalid_ranges() {
        let whirlpool = whirlpool(64);
        let mut p = position(0, 0, 0);
        assert_eq!(
            reset_error(&mut p, &whirlpool, -64, 64, true),
            whirlpool_error_code(ErrorCode::SameTickRangeNotAllowed)
        );
        let max_usable = MAX_TICK_INDEX / 64 * 64;
        let min_usable = MIN_TICK_INDEX / 64 * 64;
        for (lower, upper) in [
            (0, 0),               // empty range
            (128, 0),             // inverted
            (1, 128),             // not a multiple of tick spacing
            (0, 127),             // not a multiple of tick spacing
            (min_usable - 64, 0), // below MIN_TICK_INDEX
            (0, max_usable + 64), // above MAX_TICK_INDEX
        ] {
            assert_eq!(
                reset_error(&mut p, &whirlpool, lower, upper, true),
                whirlpool_error_code(ErrorCode::InvalidTickIndex),
                "range [{lower}, {upper})"
            );
        }
        // nothing was written by the failed attempts
        assert_eq!(p.get().tick_lower_index(), -64);
        assert_eq!(p.get().tick_upper_index(), 64);
    }

    #[test]
    fn full_range_only_pools_accept_only_full_range() {
        let tick_spacing = FULL_RANGE_ONLY_TICK_SPACING_THRESHOLD;
        let whirlpool = whirlpool(tick_spacing);
        let (lower, upper) = Tick::full_range_indexes(tick_spacing);
        let mut p = memory_mapped_position(&crate::state::Position::default());
        assert_eq!(
            reset_error(&mut p, &whirlpool, 0, upper, true),
            whirlpool_error_code(ErrorCode::FullRangeOnlyPool)
        );
        expect_ok(
            p.get_mut()
                .reset_position_range(whirlpool.get(), lower, upper, true),
        );
    }
}
