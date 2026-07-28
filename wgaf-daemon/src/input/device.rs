//! Low-level `/dev/uinput` virtual device: creation via the kernel's
//! `UI_SET_*`/`UI_DEV_SETUP`/`UI_DEV_CREATE` ioctls, and raw `struct
//! input_event` emission. One combined keyboard+pointer device (matching
//! `ydotoold`'s approach and this daemon's current scope), not two separate
//! devices.
//!
//! This module owns all the `unsafe` in the `input` module. Everything
//! above it (`keyboard.rs`, `mouse.rs`, `mod.rs`) only ever calls
//! [`UinputDevice::key_event`]/[`UinputDevice::rel_event`]/
//! [`UinputDevice::sync`], which are safe, ordinary Rust.
//!
//! The `UI_SET_EVBIT`/`UI_SET_KEYBIT`/`UI_DEV_SETUP`/`UI_DEV_CREATE` ioctl
//! request numbers are computed from the same `_IOC`/`_IOW`/`_IO` formula
//! the kernel headers use (see the `ioc`/`io`/`iow` `const fn`s below)
//! rather than hand-copied as magic numbers — `tests::ioctl_numbers_match_known_kernel_constants`
//! cross-checks the result against the literal values documented in
//! `<linux/uinput.h>` (`0x40045564`, `0x5501`, `0x405c5503`), which were
//! also confirmed against a real kernel in this environment.

use std::fs::{File, OpenOptions};
use std::io;
use std::mem::size_of;
use std::os::fd::AsRawFd;

use crate::input::InputError;
use crate::input::codes::{ALL_KEYS, EV_KEY, EV_REL, EV_SYN, REL_HWHEEL, REL_WHEEL, REL_X, REL_Y};

/// Path to the kernel's `uinput` control device. Not currently
/// configurable — unlike `Config::extension_bus_name` for the Windows1/
/// extension boundary, there's no equivalent stub/fake for `uinput` itself
/// (the daemon-side test suite exercises the *real* device at this real
/// path, since it's confirmed writable in the target environment).
const UINPUT_PATH: &str = "/dev/uinput";

/// Vendor/product/version reported in the device's `input_id` — arbitrary
/// but wgaf's own (not `ydotool`'s `0x2333`/`0x6666`), bus type `BUS_VIRTUAL`.
const BUS_VIRTUAL: u16 = 0x06;
const VENDOR_ID: u16 = 0x57ae; // "wgaf"-ish, arbitrary
const PRODUCT_ID: u16 = 0x0001;
const VERSION: u16 = 1;

// ---------------------------------------------------------------------------
// ioctl request-number formula, mirroring <asm-generic/ioctl.h>. Computed
// rather than hand-copied so the derivation is auditable (and self-checked
// by a test) instead of resting on transcription accuracy alone.
// ---------------------------------------------------------------------------

const IOC_NRBITS: u32 = 8;
const IOC_TYPEBITS: u32 = 8;
const IOC_SIZEBITS: u32 = 14;

const IOC_NRSHIFT: u32 = 0;
const IOC_TYPESHIFT: u32 = IOC_NRSHIFT + IOC_NRBITS;
const IOC_SIZESHIFT: u32 = IOC_TYPESHIFT + IOC_TYPEBITS;
const IOC_DIRSHIFT: u32 = IOC_SIZESHIFT + IOC_SIZEBITS;

const IOC_NONE: u32 = 0;
const IOC_WRITE: u32 = 1;

const fn ioc(dir: u32, ty: u32, nr: u32, size: u32) -> u32 {
    (dir << IOC_DIRSHIFT) | (ty << IOC_TYPESHIFT) | (nr << IOC_NRSHIFT) | (size << IOC_SIZESHIFT)
}

const fn io(ty: u32, nr: u32) -> u32 {
    ioc(IOC_NONE, ty, nr, 0)
}

const fn iow(ty: u32, nr: u32, size: u32) -> u32 {
    ioc(IOC_WRITE, ty, nr, size)
}

/// `UINPUT_IOCTL_BASE` from `<linux/uinput.h>` — the ASCII code for `'U'`.
const UINPUT_IOCTL_BASE: u32 = 0x55;

const UI_SET_EVBIT: u32 = iow(UINPUT_IOCTL_BASE, 100, size_of::<libc::c_int>() as u32);
const UI_SET_KEYBIT: u32 = iow(UINPUT_IOCTL_BASE, 101, size_of::<libc::c_int>() as u32);
const UI_SET_RELBIT: u32 = iow(UINPUT_IOCTL_BASE, 102, size_of::<libc::c_int>() as u32);
const UI_DEV_CREATE: u32 = io(UINPUT_IOCTL_BASE, 1);
const UI_DEV_DESTROY: u32 = io(UINPUT_IOCTL_BASE, 2);
const UI_DEV_SETUP: u32 = iow(UINPUT_IOCTL_BASE, 3, size_of::<libc::uinput_setup>() as u32);

/// Owns the open `/dev/uinput` file descriptor for one combined virtual
/// keyboard+pointer device, from `UI_DEV_CREATE` to `UI_DEV_DESTROY`.
pub(crate) struct UinputDevice {
    file: File,
}

/// Reports whether `/dev/uinput` is currently openable for writing, without
/// creating anything.
///
/// This is the health probe behind `org.wgaf.Daemon1.Status`, and the
/// distinction from [`UinputDevice::create`] is the whole point: it opens the
/// control device and immediately drops the handle, issuing **no ioctls at
/// all**. No `UI_DEV_CREATE` means no virtual device appears in
/// `/proc/bus/input/devices`, so asking "could input synthesis work?" never
/// becomes "input synthesis has now started". A status query that registers a
/// kernel device as a side effect would also perturb
/// `wgaf-daemon/tests/input.rs`, which asserts on exactly that file.
pub(crate) fn probe_access() -> Result<(), InputError> {
    OpenOptions::new()
        .write(true)
        .open(UINPUT_PATH)
        .map(|_| ())
        .map_err(|e| device_unavailable(UINPUT_PATH, &e))
}

impl UinputDevice {
    /// Opens `/dev/uinput`, registers every key/button/axis in
    /// [`crate::input::codes::ALL_KEYS`] plus the relative axes this daemon
    /// synthesizes, and creates the device reporting `device_name` to the
    /// kernel (normally [`crate::input::DEFAULT_DEVICE_NAME`], overridable
    /// for test isolation — see that constant's doc comment).
    /// Blocking/synchronous — callers run this via
    /// `tokio::task::spawn_blocking` (see `mod.rs`), since it's a handful of
    /// `ioctl`s during startup, not a hot path.
    pub(crate) fn create(device_name: &str) -> Result<Self, InputError> {
        let file = OpenOptions::new()
            .write(true)
            .open(UINPUT_PATH)
            .map_err(|e| device_unavailable(UINPUT_PATH, &e))?;
        let fd = file.as_raw_fd();

        // EV_KEY + every key/button code we support.
        ioctl_value(fd, UI_SET_EVBIT, EV_KEY as libc::c_ulong)
            .map_err(|e| device_unavailable(UINPUT_PATH, &e))?;
        for &key in ALL_KEYS {
            ioctl_value(fd, UI_SET_KEYBIT, key as libc::c_ulong)
                .map_err(|e| device_unavailable(UINPUT_PATH, &e))?;
        }

        // EV_REL + the relative axes we emit (pointer motion and scroll).
        ioctl_value(fd, UI_SET_EVBIT, EV_REL as libc::c_ulong)
            .map_err(|e| device_unavailable(UINPUT_PATH, &e))?;
        for &axis in &[REL_X, REL_Y, REL_WHEEL, REL_HWHEEL] {
            ioctl_value(fd, UI_SET_RELBIT, axis as libc::c_ulong)
                .map_err(|e| device_unavailable(UINPUT_PATH, &e))?;
        }

        // Truncates silently if `device_name` is longer than
        // `UINPUT_MAX_NAME_SIZE - 1` bytes (leaving room for the implicit
        // NUL) — `zip` stops at the shorter iterator, so this can't
        // overflow `name`'s fixed-size array.
        let mut name = [0i8; libc::UINPUT_MAX_NAME_SIZE];
        for (dst, src) in name.iter_mut().zip(
            device_name
                .as_bytes()
                .iter()
                .take(libc::UINPUT_MAX_NAME_SIZE - 1),
        ) {
            *dst = *src as i8;
        }
        let setup = libc::uinput_setup {
            id: libc::input_id {
                bustype: BUS_VIRTUAL,
                vendor: VENDOR_ID,
                product: PRODUCT_ID,
                version: VERSION,
            },
            name,
            ff_effects_max: 0,
        };
        ioctl_ptr(fd, UI_DEV_SETUP, &setup).map_err(|e| device_unavailable(UINPUT_PATH, &e))?;
        ioctl_none(fd, UI_DEV_CREATE).map_err(|e| device_unavailable(UINPUT_PATH, &e))?;

        Ok(Self { file })
    }

    /// Emits one `EV_KEY` event (`value`: `1` = press, `0` = release) and
    /// syncs. Every key/button synthesis (keyboard keys and mouse buttons
    /// alike, since both live in the `EV_KEY` type) goes through this.
    pub(crate) fn key_event(&mut self, code: u16, value: i32) -> Result<(), InputError> {
        self.emit(EV_KEY, code, value)?;
        self.sync()
    }

    /// Emits one `EV_REL` event and syncs — used for both pointer motion
    /// (`REL_X`/`REL_Y`) and scrolling (`REL_WHEEL`/`REL_HWHEEL`).
    pub(crate) fn rel_event(&mut self, code: u16, value: i32) -> Result<(), InputError> {
        self.emit(EV_REL, code, value)?;
        self.sync()
    }

    /// Emits `REL_X` and `REL_Y` as one `SYN_REPORT`-terminated batch, so
    /// readers see a single atomic move rather than two separate motions.
    pub(crate) fn rel_move(&mut self, dx: i32, dy: i32) -> Result<(), InputError> {
        self.emit(EV_REL, REL_X, dx)?;
        self.emit(EV_REL, REL_Y, dy)?;
        self.sync()
    }

    fn emit(&mut self, type_: u16, code: u16, value: i32) -> Result<(), InputError> {
        use std::io::Write;

        let event = libc::input_event {
            time: libc::timeval {
                tv_sec: 0,
                tv_usec: 0,
            },
            type_,
            code,
            value,
        };
        // SAFETY: `libc::input_event` is `#[repr(C)]` with no padding bytes
        // that matter for a `write()` — reinterpreting it as its own byte
        // representation to hand to `write_all` is exactly what writing a C
        // struct to a device file requires.
        let bytes = unsafe {
            std::slice::from_raw_parts(
                &event as *const libc::input_event as *const u8,
                size_of::<libc::input_event>(),
            )
        };
        self.file.write_all(bytes).map_err(InputError::Io)
    }

    fn sync(&mut self) -> Result<(), InputError> {
        self.emit(EV_SYN, crate::input::codes::SYN_REPORT, 0)
    }
}

impl Drop for UinputDevice {
    fn drop(&mut self) {
        // Best-effort: if this fails there's nothing more to do (the fd is
        // about to be closed via `File`'s own `Drop` regardless), but do try
        // so the kernel promptly removes the device node rather than
        // leaving it until the fd is reclaimed.
        let _ = ioctl_none(self.file.as_raw_fd(), UI_DEV_DESTROY);
    }
}

fn device_unavailable(path: &str, source: &io::Error) -> InputError {
    InputError::DeviceUnavailable {
        path: path.to_string(),
        reason: source.to_string(),
    }
}

/// Issues an ioctl whose argument is a small value passed by-value (not a
/// pointer) — `UI_SET_EVBIT`/`UI_SET_KEYBIT`/`UI_SET_RELBIT`'s calling
/// convention, despite their `_IOW` classification suggesting "write a
/// pointed-to `int`": the kernel's `uinput` driver reads this one directly
/// out of the `ioctl` syscall's `arg` register. Passed as `c_ulong`
/// (pointer-width) rather than `c_int` since `libc::ioctl`'s C variadic
/// signature reads the vararg at native word size regardless of the
/// ioctl's nominal argument type.
fn ioctl_value(fd: libc::c_int, request: u32, value: libc::c_ulong) -> io::Result<()> {
    // SAFETY: `request` is one of our own computed `UI_SET_*` constants and
    // `value` is a plain integer the kernel driver reads by value — no
    // memory is dereferenced on either side of this call.
    let ret = unsafe { libc::ioctl(fd, request as libc::Ioctl, value) };
    if ret < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// Issues an ioctl whose argument is a pointer to a struct (`UI_DEV_SETUP`).
fn ioctl_ptr<T>(fd: libc::c_int, request: u32, value: &T) -> io::Result<()> {
    // SAFETY: `value` is a valid, live reference for the duration of this
    // call, and `request` (`UI_DEV_SETUP`) is documented to read exactly
    // `size_of::<T>()` bytes from it (the size baked into the ioctl number
    // itself via `iow`'s `size` argument matches `T` = `libc::uinput_setup`).
    let ret = unsafe { libc::ioctl(fd, request as libc::Ioctl, value as *const T) };
    if ret < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

/// Issues an ioctl with no argument (`UI_DEV_CREATE`/`UI_DEV_DESTROY`).
fn ioctl_none(fd: libc::c_int, request: u32) -> io::Result<()> {
    // SAFETY: no argument is passed or read for these two request codes.
    let ret = unsafe { libc::ioctl(fd, request as libc::Ioctl) };
    if ret < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Cross-checks the computed ioctl request numbers against the literal
    /// values documented in `<linux/uinput.h>` (stable across kernel
    /// versions) — confirmed against a real kernel in this environment.
    /// This guards against a mistake in the `ioc`/`io`/`iow` formula itself,
    /// independent of whether `/dev/uinput` is available to actually
    /// exercise it.
    #[test]
    fn ioctl_numbers_match_known_kernel_constants() {
        assert_eq!(UI_SET_EVBIT, 0x4004_5564);
        assert_eq!(UI_SET_KEYBIT, 0x4004_5565);
        assert_eq!(UI_SET_RELBIT, 0x4004_5566);
        assert_eq!(UI_DEV_CREATE, 0x5501);
        assert_eq!(UI_DEV_DESTROY, 0x5502);
        assert_eq!(UI_DEV_SETUP, 0x405c_5503);
    }

    #[test]
    fn uinput_setup_size_matches_expected_92_bytes() {
        // struct input_id (4x u16 = 8 bytes) + name[80] + u32 = 92 bytes.
        // UI_DEV_SETUP's encoded size only matches a real kernel's
        // expectations if this holds.
        assert_eq!(size_of::<libc::uinput_setup>(), 92);
    }
}
