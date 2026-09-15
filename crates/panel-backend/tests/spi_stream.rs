//! Drives `Panel` with fake SPI/GPIO and checks the exact byte stream the
//! ED2208 controller would receive.

use std::cell::RefCell;
use std::convert::Infallible;
use std::rc::Rc;

use embedded_hal::delay::DelayNs;
use embedded_hal::digital::{ErrorType as DigitalErrorType, InputPin, OutputPin};
use embedded_hal::spi::{ErrorType as SpiErrorType, Operation, SpiDevice};
use panel_backend::Panel;
use render::{Frame, Spectra6, FRAME_BYTES};

/// Everything written over SPI, tagged with the DC level at the time
/// (`false` = command byte, `true` = data bytes).
#[derive(Default)]
struct Log {
    dc_high: bool,
    writes: Vec<(bool, Vec<u8>)>,
    resets: usize,
}

#[derive(Clone)]
struct Shared(Rc<RefCell<Log>>);

struct FakeSpi(Shared);
impl SpiErrorType for FakeSpi {
    type Error = Infallible;
}
impl SpiDevice for FakeSpi {
    fn transaction(&mut self, ops: &mut [Operation<'_, u8>]) -> Result<(), Infallible> {
        let mut log = self.0 .0.borrow_mut();
        let dc = log.dc_high;
        for op in ops {
            match op {
                Operation::Write(buf) => log.writes.push((dc, buf.to_vec())),
                Operation::Read(buf) => buf.fill(0),
                Operation::Transfer(r, _) => r.fill(0),
                Operation::TransferInPlace(buf) => buf.fill(0),
                Operation::DelayNs(_) => {}
            }
        }
        Ok(())
    }
}

struct DcPin(Shared);
impl DigitalErrorType for DcPin {
    type Error = Infallible;
}
impl OutputPin for DcPin {
    fn set_low(&mut self) -> Result<(), Infallible> {
        self.0 .0.borrow_mut().dc_high = false;
        Ok(())
    }
    fn set_high(&mut self) -> Result<(), Infallible> {
        self.0 .0.borrow_mut().dc_high = true;
        Ok(())
    }
}

struct RstPin(Shared);
impl DigitalErrorType for RstPin {
    type Error = Infallible;
}
impl OutputPin for RstPin {
    fn set_low(&mut self) -> Result<(), Infallible> {
        self.0 .0.borrow_mut().resets += 1;
        Ok(())
    }
    fn set_high(&mut self) -> Result<(), Infallible> {
        Ok(())
    }
}

/// The panel drives BUSY low while busy; an idle panel reads high.
struct IdleBusy;
impl DigitalErrorType for IdleBusy {
    type Error = Infallible;
}
impl InputPin for IdleBusy {
    fn is_high(&mut self) -> Result<bool, Infallible> {
        Ok(true)
    }
    fn is_low(&mut self) -> Result<bool, Infallible> {
        Ok(false)
    }
}

struct NoDelay;
impl DelayNs for NoDelay {
    fn delay_ns(&mut self, _: u32) {}
}

fn panel() -> (Panel<FakeSpi, DcPin, RstPin, IdleBusy>, Shared) {
    let shared = Shared(Rc::new(RefCell::new(Log::default())));
    let panel = Panel::new(
        FakeSpi(shared.clone()),
        DcPin(shared.clone()),
        RstPin(shared.clone()),
        IdleBusy,
    );
    (panel, shared)
}

/// Flattens the log into `(command, data-bytes-that-followed)` pairs.
fn commands(log: &Log) -> Vec<(u8, Vec<u8>)> {
    let mut out: Vec<(u8, Vec<u8>)> = Vec::new();
    for (is_data, bytes) in &log.writes {
        if *is_data {
            out.last_mut()
                .expect("data before any command")
                .1
                .extend(bytes);
        } else {
            assert_eq!(bytes.len(), 1, "commands are single bytes");
            out.push((bytes[0], Vec::new()));
        }
    }
    out
}

#[test]
fn init_resets_then_sends_ed2208_sequence_and_powers_on() {
    let (mut p, shared) = panel();
    p.init(&mut NoDelay).unwrap();
    let log = shared.0.borrow();
    assert!(log.resets >= 1, "hardware reset pulsed");
    let cmds = commands(&log);
    let codes: Vec<u8> = cmds.iter().map(|(c, _)| *c).collect();
    // First register write is CMDH with its magic unlock sequence.
    assert_eq!(cmds[0], (0xAA, vec![0x49, 0x55, 0x20, 0x08, 0x09, 0x18]));
    // Resolution: 800 x 480.
    assert!(
        cmds.contains(&(0x61, vec![0x03, 0x20, 0x01, 0xE0])),
        "{cmds:?}"
    );
    // Power on is the last thing init does.
    assert_eq!(codes.last(), Some(&0x04));
    for expected in [
        0x00, 0x01, 0x03, 0x05, 0x06, 0x08, 0x30, 0x50, 0x60, 0x84, 0xE3,
    ] {
        assert!(codes.contains(&expected), "missing command {expected:#04x}");
    }
}

#[test]
fn show_streams_the_frame_then_refreshes() {
    let (mut p, shared) = panel();
    p.init(&mut NoDelay).unwrap();
    shared.0.borrow_mut().writes.clear();

    let mut frame = Box::new(Frame::new());
    frame.set_pixel(0, 0, Spectra6::Red);
    frame.set_pixel(799, 479, Spectra6::Green);
    p.show(frame.as_bytes(), &mut NoDelay).unwrap();

    let log = shared.0.borrow();
    let cmds = commands(&log);
    assert_eq!(
        cmds.len(),
        2,
        "{:?}",
        cmds.iter().map(|(c, d)| (c, d.len())).collect::<Vec<_>>()
    );
    let (cmd, data) = &cmds[0];
    assert_eq!(*cmd, 0x10, "data start transmission");
    assert_eq!(data.len(), FRAME_BYTES);
    assert_eq!(
        &data[..],
        frame.as_bytes(),
        "frame bytes pass through untouched"
    );
    assert_eq!(data[0], 0x31, "red then white, high nibble first");
    assert_eq!(data[FRAME_BYTES - 1], 0x16);
    assert_eq!(cmds[1], (0x12, vec![0x00]), "display refresh");
}

#[test]
fn short_or_long_frames_are_rejected_before_any_spi_traffic() {
    let (mut p, shared) = panel();
    p.init(&mut NoDelay).unwrap();
    shared.0.borrow_mut().writes.clear();

    assert!(p.show(&[0x11; FRAME_BYTES - 1], &mut NoDelay).is_err());
    assert!(p.show(&[0x11; FRAME_BYTES + 1], &mut NoDelay).is_err());
    assert!(p.show(&[], &mut NoDelay).is_err());
    assert!(shared.0.borrow().writes.is_empty());
}

#[test]
fn clear_fills_with_the_packed_colour() {
    let (mut p, shared) = panel();
    p.init(&mut NoDelay).unwrap();
    shared.0.borrow_mut().writes.clear();
    p.clear(Spectra6::Yellow, &mut NoDelay).unwrap();
    let log = shared.0.borrow();
    let cmds = commands(&log);
    assert_eq!(cmds[0].0, 0x10);
    assert_eq!(cmds[0].1.len(), FRAME_BYTES);
    assert!(cmds[0].1.iter().all(|&b| b == 0x22));
    assert_eq!(cmds[1], (0x12, vec![0x00]));
}

#[test]
fn sleep_powers_off_then_deep_sleeps() {
    let (mut p, shared) = panel();
    p.init(&mut NoDelay).unwrap();
    shared.0.borrow_mut().writes.clear();
    p.sleep(&mut NoDelay).unwrap();
    let log = shared.0.borrow();
    assert_eq!(
        commands(&log),
        vec![(0x02, vec![0x00]), (0x07, vec![0xA5])],
        "power off, then deep sleep with its check code"
    );
}
