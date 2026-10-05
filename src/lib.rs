#![no_std]

pub const SAMPLE_RATE: u32 = 48_000;
pub const BLOCK_FRAMES: usize = 480;
pub const BLOCK_WORDS: usize = BLOCK_FRAMES * 2;
pub const PIO_CYCLES_PER_FRAME: u32 = 256;

/// One SM owns BCLK (side bit 0), LRCLK (side bit 1), TX and RX.
/// 32-bit MSB-first slots; LRCLK changes at the preceding slot's LSB,
/// giving the standard I2S one-bit delay. Each bit takes four PIO cycles:
/// OUT + delay at BCLK low, IN at high, then JMP/SET at high.
/// Autopull/autopush thresholds must both be 32.
pub fn i2s_program() -> pio::Program<32> {
    use pio::{Assembler, InSource, JmpCondition, OutDestination, SetDestination, SideSet};
    let mut a = Assembler::<32>::new_with_side_set(SideSet::new(false, 2, false));
    let mut start = a.label();
    let mut left = a.label();
    let mut right = a.label();
    let mut end = a.label();
    a.set_with_side_set(SetDestination::X, 30, 0);
    a.bind(&mut start);
    a.bind(&mut left);
    a.out_with_delay_and_side_set(OutDestination::PINS, 1, 1, 0);
    a.in_with_side_set(InSource::PINS, 1, 1);
    a.jmp_with_side_set(JmpCondition::XDecNonZero, &mut left, 1);
    a.out_with_delay_and_side_set(OutDestination::PINS, 1, 1, 2);
    a.in_with_side_set(InSource::PINS, 1, 3);
    a.set_with_side_set(SetDestination::X, 30, 3);
    a.bind(&mut right);
    a.out_with_delay_and_side_set(OutDestination::PINS, 1, 1, 2);
    a.in_with_side_set(InSource::PINS, 1, 3);
    a.jmp_with_side_set(JmpCondition::XDecNonZero, &mut right, 3);
    a.out_with_delay_and_side_set(OutDestination::PINS, 1, 1, 0);
    a.in_with_side_set(InSource::PINS, 1, 1);
    a.set_with_side_set(SetDestination::X, 30, 1);
    a.bind(&mut end);
    a.assemble_with_wrap(end, start)
}

/// ICS-43434 has 24 significant bits at the top of the 32-bit slot.
/// Arithmetic shift preserves sign and discards the undriven padding.
pub fn microphone_pcm16(slot: u32) -> i16 {
    ((slot as i32) >> 16) as i16
}

pub fn speaker_slot(sample: i16) -> u32 {
    ((sample as i32) << 16) as u32
}

/// Integer 1 kHz triangle wave at 48 kHz, peak 512 / 32768 (~1.6%).
/// Explicit opt-in only; the default firmware transmits silence.
#[derive(Default)]
pub struct TestTone {
    phase: u32,
}
impl TestTone {
    pub fn next_sample(&mut self) -> i16 {
        let p = self.phase as i32;
        self.phase = (self.phase + 1) % 48;
        let ramp = if p < 24 { p } else { 48 - p };
        ((ramp - 12) * 512 / 12) as i16
    }
}

#[derive(Default)]
pub struct CaptureStats {
    pub peak: u32,
    pub sum_squares: u64,
    pub samples: u32,
    pub clipped: u32,
}
impl CaptureStats {
    pub fn add(&mut self, sample: i16) {
        let value = sample as i32;
        self.peak = self.peak.max(value.unsigned_abs());
        self.sum_squares += (value as i64 * value as i64) as u64;
        self.samples += 1;
        self.clipped += u32::from(sample == i16::MIN || sample == i16::MAX);
    }
    pub fn rms(&self) -> u32 {
        if self.samples == 0 {
            return 0;
        }
        let n = self.sum_squares / self.samples as u64;
        let (mut lo, mut hi) = (0u64, 32769u64);
        while lo + 1 < hi {
            let mid = (lo + hi) / 2;
            if mid * mid <= n {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        lo as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    // Execute the actual assembled instructions. This verifies the loop count,
    // wrap boundary, side-set timing, and channel delay rather than a second
    // hand-written model of the intended waveform. FIFOs are assumed ready.
    #[test]
    fn assembled_i2s_has_64_clocks_and_one_bit_delay() {
        use pio::{Instruction, InstructionOperands as Op, JmpCondition, SetDestination};
        let program = i2s_program();
        let mut pc = 0usize;
        let mut x = 0u32;
        let mut cycle = 0usize;
        let mut prev_side = 0u8;
        let mut rising = 0usize;
        let mut words = 0usize;
        let mut bits = 0usize;
        let mut last_ws_change = None;
        let mut previous_rising = None;
        let mut high_started = None;
        for _ in 0..1600 {
            let instruction = Instruction::decode(program.code[pc], program.side_set).unwrap();
            let side = instruction.side_set.unwrap();
            if side & 2 != prev_side & 2 {
                assert_eq!(bits, 31, "WS must change for the preceding word's LSB");
                assert_eq!(side & 1, 0);
                last_ws_change = Some(cycle);
            }
            if side & 1 == 0 && prev_side & 1 != 0 {
                assert_eq!(cycle - high_started.unwrap(), 2);
            }
            if side & 1 != 0 && prev_side & 1 == 0 {
                if let Some(last) = previous_rising {
                    assert_eq!(cycle - last, 4);
                }
                previous_rising = Some(cycle);
                high_started = Some(cycle);
                rising += 1;
                bits += 1;
                if bits == 1 && words > 0 {
                    assert_eq!(cycle - last_ws_change.unwrap(), 6);
                    assert_eq!((side >> 1) & 1, (words % 2) as u8);
                }
                if bits == 32 {
                    bits = 0;
                    words += 1;
                }
            }
            let mut jump = None;
            match instruction.operands {
                Op::SET {
                    destination: SetDestination::X,
                    data,
                } => x = data as u32,
                Op::JMP {
                    condition: JmpCondition::XDecNonZero,
                    address,
                } => {
                    if x != 0 {
                        jump = Some(address as usize);
                    }
                    x = x.wrapping_sub(1);
                }
                Op::IN { bit_count, .. } => {
                    assert_eq!(bit_count, 1);
                    assert_eq!(side & 1, 1, "capture on rising edge");
                }
                Op::OUT { bit_count, .. } => {
                    assert_eq!(bit_count, 1);
                    assert_eq!(side & 1, 0, "update output on falling edge");
                    assert_eq!(instruction.delay, 1);
                }
                _ => panic!("unexpected instruction"),
            }
            cycle += 1 + instruction.delay as usize;
            prev_side = side;
            pc = jump.unwrap_or_else(|| {
                if pc == program.wrap.source as usize {
                    program.wrap.target as usize
                } else {
                    pc + 1
                }
            });
        }
        assert!(rising >= 512);
        assert!(words >= 16);
    }
    #[test]
    fn pcm_sign_and_padding() {
        assert_eq!(microphone_pcm16(0x7fffff00), 32767);
        assert_eq!(microphone_pcm16(0x800000ff), -32768);
        assert_eq!(microphone_pcm16(0xffffff00), -1);
        assert_eq!(speaker_slot(-32768), 0x80000000);
        assert_eq!(speaker_slot(32767), 0x7fff0000);
    }
    #[test]
    fn stats_handle_negative_full_scale() {
        let mut stats = CaptureStats::default();
        stats.add(-32768);
        stats.add(32767);
        assert_eq!(stats.peak, 32768);
        assert_eq!(stats.clipped, 2);
        assert_eq!(stats.rms(), 32767);
        assert_eq!(CaptureStats::default().rms(), 0);
    }
    #[test]
    fn tone_is_bounded_periodic_and_dc_free() {
        let mut tone = TestTone::default();
        let mut sum = 0i32;
        let first = tone.next_sample();
        sum += first as i32;
        for _ in 1..48 {
            let s = tone.next_sample();
            assert!(s.abs() <= 512);
            sum += s as i32;
        }
        assert_eq!(sum, 0);
        assert_eq!(tone.next_sample(), first);
    }
}
