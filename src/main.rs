#![no_std]
#![no_main]

use defmt_rtt as _;
use panic_probe as _;
use pi_ai_rpipico::{
    BLOCK_FRAMES, BLOCK_WORDS, CaptureStats, PIO_CYCLES_PER_FRAME, SAMPLE_RATE, TestTone,
    i2s_program, microphone_pcm16, speaker_slot,
};
use rp235x_hal::{self as hal, clocks::Clock, dma::DMAExt, pio::PIOExt};

#[unsafe(link_section = ".start_block")]
#[used]
pub static IMAGE_DEF: hal::block::ImageDef = hal::block::ImageDef::secure_exe();

#[hal::entry]
fn main() -> ! {
    let mut pac = hal::pac::Peripherals::take().unwrap();
    let mut watchdog = hal::Watchdog::new(pac.WATCHDOG);
    let clocks = hal::clocks::init_clocks_and_plls(
        12_000_000,
        pac.XOSC,
        pac.CLOCKS,
        pac.PLL_SYS,
        pac.PLL_USB,
        &mut pac.RESETS,
        &mut watchdog,
    )
    .unwrap();
    let sio = hal::Sio::new(pac.SIO);
    let pins = hal::gpio::Pins::new(
        pac.IO_BANK0,
        pac.PADS_BANK0,
        sio.gpio_bank0,
        &mut pac.RESETS,
    );
    let _bclk = pins.gpio10.into_function::<hal::gpio::FunctionPio0>();
    let _lrclk = pins.gpio11.into_function::<hal::gpio::FunctionPio0>();
    let _mic = pins
        .gpio12
        .into_pull_down_input()
        .into_function::<hal::gpio::FunctionPio0>();
    let _speaker = pins.gpio13.into_function::<hal::gpio::FunctionPio0>();
    let (mut pio, sm0, _, _, _) = pac.PIO0.split(&mut pac.RESETS);
    let installed = pio.install(&i2s_program()).unwrap();
    let divider = ((clocks.system_clock.freq().to_Hz() as u64 * 256)
        / (SAMPLE_RATE * PIO_CYCLES_PER_FRAME) as u64) as u32;
    let (mut sm, rx, tx) = hal::pio::PIOBuilder::from_installed_program(installed)
        .side_set_pin_base(10)
        .out_pins(13, 1)
        .in_pin_base(12)
        .out_shift_direction(hal::pio::ShiftDirection::Left)
        .in_shift_direction(hal::pio::ShiftDirection::Left)
        .autopull(true)
        .pull_threshold(32)
        .autopush(true)
        .push_threshold(32)
        .clock_divisor_fixed_point((divider >> 8) as u16, divider as u8)
        .build(sm0);
    sm.set_pins([
        (10, hal::pio::PinState::Low),
        (11, hal::pio::PinState::Low),
        (13, hal::pio::PinState::Low),
    ]);
    sm.set_pindirs([
        (10, hal::pio::PinDir::Output),
        (11, hal::pio::PinDir::Output),
        (12, hal::pio::PinDir::Input),
        (13, hal::pio::PinDir::Output),
    ]);

    let tx_a = cortex_m::singleton!(: [u32; BLOCK_WORDS] = [0; BLOCK_WORDS]).unwrap();
    let tx_b = cortex_m::singleton!(: [u32; BLOCK_WORDS] = [0; BLOCK_WORDS]).unwrap();
    let rx_a = cortex_m::singleton!(: [u32; BLOCK_WORDS] = [0; BLOCK_WORDS]).unwrap();
    let rx_b = cortex_m::singleton!(: [u32; BLOCK_WORDS] = [0; BLOCK_WORDS]).unwrap();
    let mut tone = TestTone::default();
    fill_output(tx_a, &mut tone, 0);
    fill_output(tx_b, &mut tone, BLOCK_FRAMES as u32);
    let dma = pac.DMA.split(&mut pac.RESETS);
    let mut output = hal::dma::double_buffer::Config::new((dma.ch0, dma.ch1), tx_a, tx)
        .start()
        .read_next(tx_b);
    let mut input = hal::dma::double_buffer::Config::new((dma.ch2, dma.ch3), rx, rx_a)
        .start()
        .write_next(rx_b);
    defmt::info!(
        "I2S diagnostic: 48kHz, GP10/11 clocks, GP12 mic, GP13 speaker; tone={}",
        cfg!(feature = "test-tone")
    );
    let _running = sm.start();
    let mut stats = CaptureStats::default();
    let mut frames = 0u32;
    let mut stall_windows = 0u32;
    loop {
        let (out_buf, next_output) = output.wait();
        fill_output(
            out_buf,
            &mut tone,
            frames.wrapping_add(2 * BLOCK_FRAMES as u32),
        );
        output = next_output.read_next(out_buf);
        let (in_buf, next_input) = input.wait();
        // Ignore the first 100ms for microphone startup and initial frame alignment.
        if frames >= SAMPLE_RATE / 10 {
            for pair in in_buf.chunks_exact(2) {
                stats.add(microphone_pcm16(pair[0]));
            }
        }
        input = next_input.write_next(in_buf);
        frames = frames.wrapping_add(BLOCK_FRAMES as u32);
        // HAL owns the PIO block. Only read/W1C the independent SM0 debug
        // flags here; no configuration or FIFO registers are accessed.
        let debug = unsafe { &*hal::pac::PIO0::ptr() }.fdebug();
        let flags = debug.read();
        if flags.rxstall().bits() & 1 != 0 || flags.txstall().bits() & 1 != 0 {
            stall_windows = stall_windows.saturating_add(1);
            debug.write(|w| unsafe { w.rxstall().bits(1).txstall().bits(1) });
        }
        if stats.samples >= SAMPLE_RATE {
            defmt::info!(
                "capture PCM16: peak={} rms={} clipped={} samples={} stall_windows={}",
                stats.peak,
                stats.rms(),
                stats.clipped,
                stats.samples,
                stall_windows
            );
            stats = CaptureStats::default();
            stall_windows = 0;
        }
    }
}

fn fill_output(buf: &mut [u32; BLOCK_WORDS], tone: &mut TestTone, start: u32) {
    for (i, pair) in buf.chunks_exact_mut(2).enumerate() {
        // A one-second quiet tone every five seconds, only in the opt-in build.
        let sample = if cfg!(feature = "test-tone")
            && start.wrapping_add(i as u32) % (SAMPLE_RATE * 5) < SAMPLE_RATE
        {
            tone.next_sample()
        } else {
            0
        };
        pair.fill(speaker_slot(sample));
    }
}

#[unsafe(link_section = ".bi_entries")]
#[used]
pub static PICOTOOL_ENTRIES: [hal::binary_info::EntryAddr; 3] = [
    hal::binary_info::rp_cargo_bin_name!(),
    hal::binary_info::rp_cargo_version!(),
    hal::binary_info::rp_program_description!(c"Pico 2 W I2S audio diagnostic"),
];
