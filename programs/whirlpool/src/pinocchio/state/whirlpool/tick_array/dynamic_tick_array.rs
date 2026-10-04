use super::super::super::{BytesI32, BytesU128, Pubkey};
use super::{tick::MemoryMappedTick, TickArray, TickUpdate, TICK_ARRAY_SIZE_USIZE};
use crate::pinocchio::Result;

const DYNAMIC_TICK_INITIALIZED_LEN: usize = 113;
const DYNAMIC_TICK_UNINITIALIZED_LEN: usize = 1;
const TICKS_MAX_USIZE: usize = DYNAMIC_TICK_INITIALIZED_LEN * TICK_ARRAY_SIZE_USIZE;

#[repr(C)]
pub struct MemoryMappedDynamicTickArray {
    discriminator: [u8; 8],

    start_tick_index: BytesI32,
    whirlpool: Pubkey,
    tick_bitmap: BytesU128,
    ticks: [u8; TICKS_MAX_USIZE],
}

impl TickArray for MemoryMappedDynamicTickArray {
    fn is_variable_size(&self) -> bool {
        true
    }

    fn start_tick_index(&self) -> i32 {
        i32::from_le_bytes(self.start_tick_index)
    }

    fn whirlpool(&self) -> &Pubkey {
        &self.whirlpool
    }

    fn get_tick(&self, tick_index: i32, tick_spacing: u16) -> Result<&MemoryMappedTick> {
        let tick_offset = match self.check_is_usable_tick_and_get_offset(tick_index, tick_spacing) {
            Some(offset) => offset,
            None => {
                return Err(crate::errors::ErrorCode::TickNotFound.into());
            }
        };
        let byte_offset = self.byte_offset(tick_offset)?;

        if self.ticks[byte_offset] == 0 {
            Ok(&super::tick::STATIC_ZEROED_MEMORY_MAPPED_TICK)
        } else {
            let tick_bytes = &self.ticks[byte_offset..byte_offset + DYNAMIC_TICK_INITIALIZED_LEN];
            let tick_ptr = tick_bytes.as_ptr() as *const MemoryMappedTick;
            unsafe { Ok(&*tick_ptr) }
        }
    }

    fn update_tick(
        &mut self,
        tick_index: i32,
        tick_spacing: u16,
        update: &TickUpdate,
    ) -> Result<()> {
        let tick_offset = match self.check_is_usable_tick_and_get_offset(tick_index, tick_spacing) {
            Some(offset) => offset,
            None => {
                return Err(crate::errors::ErrorCode::TickNotFound.into());
            }
        };
        let byte_offset = self.byte_offset(tick_offset)?;

        let tick_initialized = self.ticks[byte_offset] != 0;

        // If the tick needs to be initialized, we need to right-shift everything after byte_offset by DynamicTickData::LEN
        if !tick_initialized && update.initialized {
            let shift_data = &mut self.ticks[byte_offset..];
            shift_data.rotate_right(crate::state::DynamicTickData::LEN);

            // sync bitmap
            self.update_tick_bitmap(tick_offset, true);
        }

        // If the tick needs to be uninitialized, we need to left-shift everything after byte_offset by DynamicTickData::LEN
        if tick_initialized && !update.initialized {
            let shift_data = &mut self.ticks[byte_offset..];
            shift_data.rotate_left(crate::state::DynamicTickData::LEN);

            // sync bitmap
            self.update_tick_bitmap(tick_offset, false);
        }

        // Update the tick data at byte_offset
        if !update.initialized {
            // If the tick is being uninitialized, we are done
            self.ticks[byte_offset] = 0;
        } else {
            // map MemoryMappedTick and update
            let tick_bytes = &mut self.ticks
                [byte_offset..byte_offset + crate::state::DynamicTick::INITIALIZED_LEN];
            let tick_ptr = tick_bytes.as_mut_ptr() as *mut MemoryMappedTick;
            let tick = unsafe { &mut *tick_ptr };
            tick.update(update);
        }

        Ok(())
    }
}

impl MemoryMappedDynamicTickArray {
    fn byte_offset(&self, tick_offset: usize) -> Result<usize> {
        let tick_bitmap = self.tick_bitmap();
        let mask = (1u128 << tick_offset) - 1;
        let initialized_ticks = (tick_bitmap & mask).count_ones() as usize;
        let uninitialized_ticks = tick_offset - initialized_ticks;

        let offset = initialized_ticks * DYNAMIC_TICK_INITIALIZED_LEN
            + uninitialized_ticks * DYNAMIC_TICK_UNINITIALIZED_LEN;
        Ok(offset)
    }

    fn tick_bitmap(&self) -> u128 {
        u128::from_le_bytes(self.tick_bitmap)
    }

    fn update_tick_bitmap(&mut self, tick_offset: usize, initialized: bool) {
        let mut tick_bitmap = self.tick_bitmap();
        if initialized {
            tick_bitmap |= 1 << tick_offset;
        } else {
            tick_bitmap &= !(1 << tick_offset);
        }
        self.tick_bitmap = tick_bitmap.to_le_bytes();
    }
}

/// Byte-for-byte differential test of the pinocchio dynamic tick array against the Anchor
/// `DynamicTickArrayLoader`, driven through the same resize/update ordering the handlers use.
#[cfg(test)]
mod anchor_parity_tests {
    use super::*;
    use crate::pinocchio::state::whirlpool::tick_array::TICK_ARRAY_SIZE;
    use crate::pinocchio::test_utils::Rng;
    use crate::state::{
        DynamicTickArray, DynamicTickArrayLoader, Tick, TickArrayType,
        TickUpdate as AnchorTickUpdate,
    };
    use anchor_lang::Discriminator;

    const STALE: u8 = 0xAB;
    const GROWTH: usize = crate::state::DynamicTickData::LEN;

    /// An account buffer with a logical length, mimicking the runtime's realloc padding.
    struct Account {
        bytes: Vec<u8>,
        len: usize,
    }

    impl Account {
        fn new(start_tick_index: i32) -> Self {
            // The Anchor loader is a fixed MAX_LEN struct loaded at data[8..], so it touches
            // 8 bytes beyond MAX_LEN (realloc padding on chain); give the buffer room for that.
            let mut bytes = vec![0u8; DynamicTickArray::MAX_LEN + 8];
            bytes[..8].copy_from_slice(DynamicTickArray::DISCRIMINATOR);
            bytes[8..12].copy_from_slice(&start_tick_index.to_le_bytes());
            Self {
                bytes,
                len: DynamicTickArray::MIN_LEN,
            }
        }

        // pinocchio `resize` zero-fills on growth and leaves truncated bytes untouched on
        // shrink; simulate the worst case by filling truncated bytes with garbage.
        fn grow(&mut self) {
            self.bytes[self.len..self.len + GROWTH].fill(0);
            self.len += GROWTH;
        }

        fn shrink(&mut self) {
            self.len -= GROWTH;
            self.bytes[self.len..self.len + GROWTH].fill(STALE);
        }

        fn pino(&mut self) -> &mut MemoryMappedDynamicTickArray {
            assert_eq!(
                core::mem::size_of::<MemoryMappedDynamicTickArray>(),
                DynamicTickArray::MAX_LEN
            );
            unsafe { &mut *(self.bytes.as_mut_ptr() as *mut MemoryMappedDynamicTickArray) }
        }

        fn anchor(&mut self) -> &mut DynamicTickArrayLoader {
            DynamicTickArrayLoader::load_mut(&mut self.bytes[8..])
        }
    }

    fn random_update(rng: &mut Rng) -> TickUpdate {
        if rng.below(3) == 0 {
            return TickUpdate::default();
        }
        TickUpdate {
            initialized: true,
            liquidity_net: rng.i128(),
            liquidity_gross: 1 + rng.liquidity(),
            fee_growth_outside_a: rng.u128(),
            fee_growth_outside_b: rng.u128(),
            reward_growths_outside: [rng.u128(), rng.u128(), rng.u128()],
        }
    }

    fn to_anchor(update: &TickUpdate) -> AnchorTickUpdate {
        AnchorTickUpdate {
            initialized: update.initialized,
            liquidity_net: update.liquidity_net,
            liquidity_gross: update.liquidity_gross,
            fee_growth_outside_a: update.fee_growth_outside_a,
            fee_growth_outside_b: update.fee_growth_outside_b,
            reward_growths_outside: update.reward_growths_outside,
        }
    }

    fn assert_same_tick(pino: &MemoryMappedTick, anchor: &Tick) {
        // copy out of the packed struct before comparing
        let (initialized, liquidity_net, liquidity_gross) = (
            anchor.initialized,
            anchor.liquidity_net,
            anchor.liquidity_gross,
        );
        let (fee_a, fee_b, rewards) = (
            anchor.fee_growth_outside_a,
            anchor.fee_growth_outside_b,
            anchor.reward_growths_outside,
        );
        assert_eq!(pino.initialized(), initialized);
        assert_eq!(pino.liquidity_net(), liquidity_net);
        assert_eq!(pino.liquidity_gross(), liquidity_gross);
        assert_eq!(pino.fee_growth_outside_a(), fee_a);
        assert_eq!(pino.fee_growth_outside_b(), fee_b);
        assert_eq!(pino.reward_growths_outside(), rewards);
    }

    #[test]
    fn random_init_deinit_sequences_match_anchor_byte_for_byte() {
        let mut rng = Rng::new(11);
        for _ in 0..300 {
            let tick_spacing: u16 = [1, 2, 8, 64, 128, 32768][rng.below(6) as usize];
            let array_span = TICK_ARRAY_SIZE * tick_spacing as i32;
            let start_tick_index = match rng.below(3) {
                0 => -array_span,
                1 => 0,
                _ => (rng.tick_index() / array_span) * array_span,
            };
            let mut p = Account::new(start_tick_index);
            let mut a = Account::new(start_tick_index);
            let mut expected_initialized = 0usize;

            for _ in 0..400 {
                let offset = rng.below(TICK_ARRAY_SIZE_USIZE as u64) as i32;
                let tick_index = start_tick_index + offset * tick_spacing as i32;
                let update = random_update(&mut rng);

                let was_initialized = p
                    .pino()
                    .get_tick(tick_index, tick_spacing)
                    .ok()
                    .map(|t| t.initialized());
                let anchor_was_initialized = a
                    .anchor()
                    .get_tick(tick_index, tick_spacing)
                    .ok()
                    .map(|t| t.initialized);
                assert_eq!(was_initialized, anchor_was_initialized);
                let Some(was_initialized) = was_initialized else {
                    // out of bounds tick: both must reject the update too
                    assert!(p
                        .pino()
                        .update_tick(tick_index, tick_spacing, &update)
                        .is_err());
                    assert!(a
                        .anchor()
                        .update_tick(tick_index, tick_spacing, &to_anchor(&update))
                        .is_err());
                    continue;
                };

                // Mirror the handlers: grow before writing an initialization, write before
                // shrinking on a de-initialization.
                if !was_initialized && update.initialized {
                    p.grow();
                    a.grow();
                    expected_initialized += 1;
                }
                crate::pinocchio::test_utils::expect_ok(p.pino().update_tick(
                    tick_index,
                    tick_spacing,
                    &update,
                ));
                a.anchor()
                    .update_tick(tick_index, tick_spacing, &to_anchor(&update))
                    .unwrap();
                if was_initialized && !update.initialized {
                    p.shrink();
                    a.shrink();
                    expected_initialized -= 1;
                }

                assert_eq!(p.len, a.len);
                assert_eq!(
                    p.len,
                    DynamicTickArray::MIN_LEN + expected_initialized * GROWTH,
                    "account length must track the number of initialized ticks"
                );
                assert_eq!(
                    p.pino().tick_bitmap().count_ones() as usize,
                    expected_initialized
                );
                assert_eq!(&p.bytes[..p.len], &a.bytes[..a.len]);

                for o in 0..TICK_ARRAY_SIZE {
                    let idx = start_tick_index + o * tick_spacing as i32;
                    match (
                        p.pino().get_tick(idx, tick_spacing),
                        a.anchor().get_tick(idx, tick_spacing),
                    ) {
                        (Ok(pt), Ok(at)) => assert_same_tick(pt, &at),
                        (Err(_), Err(_)) => {}
                        _ => panic!("get_tick result mismatch at {idx}"),
                    }
                }
            }
        }
    }

    #[test]
    fn nothing_past_the_account_length_is_ever_read_into_state() {
        // Fill the padding with garbage before every read; a correct array never depends on it.
        let mut rng = Rng::new(12);
        let tick_spacing = 64u16;
        let mut p = Account::new(0);
        for _ in 0..2000 {
            p.bytes[p.len..].fill(STALE);
            let offset = rng.below(TICK_ARRAY_SIZE_USIZE as u64) as i32;
            let tick_index = offset * tick_spacing as i32;
            let update = random_update(&mut rng);
            let was_initialized = p
                .pino()
                .get_tick(tick_index, tick_spacing)
                .map(|t| t.initialized());
            let was_initialized = crate::pinocchio::test_utils::expect_ok(was_initialized);
            if !was_initialized && update.initialized {
                p.grow();
            }
            p.bytes[p.len..].fill(STALE);
            crate::pinocchio::test_utils::expect_ok(p.pino().update_tick(
                tick_index,
                tick_spacing,
                &update,
            ));
            if was_initialized && !update.initialized {
                p.shrink();
            }
            let written = crate::pinocchio::test_utils::expect_ok(
                p.pino().get_tick(tick_index, tick_spacing),
            );
            assert_eq!(written.initialized(), update.initialized);
            if update.initialized {
                assert_eq!(written.liquidity_gross(), update.liquidity_gross);
                assert_eq!(
                    written.reward_growths_outside(),
                    update.reward_growths_outside
                );
            }
            assert!(
                !p.bytes[..p.len].windows(16).any(|w| w == [STALE; 16]),
                "stale padding leaked into live data"
            );
        }
    }
}
