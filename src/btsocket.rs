//! Bluetooth channels opened through the kernel's own sockets: L2CAP by its
//! PSM (AAP, Apple's accessory protocol, is on 0x1001) and RFCOMM by its
//! channel number (serial ports). BlueZ is not involved; the device must be
//! paired and in reach.
//!
//! Elsewhere than on Linux every channel fails to open, so that the code that
//! uses them is type-checked by the Windows preview build.

#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

use std::io;

/// A connected channel. Sending, receiving and closing may come from
/// different threads.
pub struct Socket {
    #[cfg(target_os = "linux")]
    fd: std::os::fd::OwnedFd,
}

/// A Bluetooth address in the order the kernel takes it: least significant
/// byte first.
fn bdaddr(address: u64) -> [u8; 6] {
    let bytes = address.to_le_bytes();
    [bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5]]
}

#[cfg(target_os = "linux")]
mod linux {
    use std::io;
    use std::mem::size_of;
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

    const BTPROTO_L2CAP: libc::c_int = 0;
    const BTPROTO_RFCOMM: libc::c_int = 3;

    /// `struct sockaddr_l2` of `<bluetooth/l2cap.h>`.
    #[repr(C)]
    pub struct SockaddrL2 {
        pub family: libc::sa_family_t,
        /// Little-endian.
        pub psm: u16,
        pub bdaddr: [u8; 6],
        pub cid: u16,
        /// 0: Bluetooth Classic.
        pub bdaddr_type: u8,
    }

    /// `struct sockaddr_rc` of `<bluetooth/rfcomm.h>`.
    #[repr(C)]
    pub struct SockaddrRc {
        pub family: libc::sa_family_t,
        pub bdaddr: [u8; 6],
        pub channel: u8,
    }

    pub fn l2cap(bdaddr: [u8; 6], psm: u16) -> io::Result<OwnedFd> {
        let address = SockaddrL2 {
            family: libc::AF_BLUETOOTH as libc::sa_family_t,
            psm: psm.to_le(),
            bdaddr,
            cid: 0,
            bdaddr_type: 0,
        };
        open(libc::SOCK_SEQPACKET, BTPROTO_L2CAP, &address)
    }

    pub fn rfcomm(bdaddr: [u8; 6], channel: u8) -> io::Result<OwnedFd> {
        let address = SockaddrRc {
            family: libc::AF_BLUETOOTH as libc::sa_family_t,
            bdaddr,
            channel,
        };
        open(libc::SOCK_STREAM, BTPROTO_RFCOMM, &address)
    }

    fn open<T>(kind: libc::c_int, protocol: libc::c_int, address: &T) -> io::Result<OwnedFd> {
        // SAFETY: a plain system call; the result is checked before use.
        let raw = unsafe { libc::socket(libc::AF_BLUETOOTH, kind | libc::SOCK_CLOEXEC, protocol) };
        if raw < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: `raw` is a socket just opened and owned by nothing else.
        let fd = unsafe { OwnedFd::from_raw_fd(raw) };
        let length = libc::socklen_t::try_from(size_of::<T>()).unwrap_or(0);
        // SAFETY: `address` points to a whole `T` of the size passed with it.
        let result = unsafe {
            libc::connect(
                fd.as_raw_fd(),
                std::ptr::from_ref(address).cast::<libc::sockaddr>(),
                length,
            )
        };
        if result < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(fd)
    }

    pub fn send(fd: &OwnedFd, data: &[u8]) -> io::Result<()> {
        // SAFETY: the buffer is valid for its length.
        let sent = unsafe {
            libc::send(
                fd.as_raw_fd(),
                data.as_ptr().cast(),
                data.len(),
                libc::MSG_NOSIGNAL,
            )
        };
        if sent < 0 {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        }
    }

    pub fn recv(fd: &OwnedFd, buffer: &mut [u8]) -> io::Result<usize> {
        // SAFETY: the buffer is valid for its length.
        let received =
            unsafe { libc::recv(fd.as_raw_fd(), buffer.as_mut_ptr().cast(), buffer.len(), 0) };
        usize::try_from(received).map_err(|_| io::Error::last_os_error())
    }

    pub fn shutdown(fd: &OwnedFd) {
        // SAFETY: a plain system call on a socket this process owns.
        unsafe {
            libc::shutdown(fd.as_raw_fd(), libc::SHUT_RDWR);
        }
    }
}

impl Socket {
    /// Opens an L2CAP channel of Bluetooth Classic.
    #[cfg(target_os = "linux")]
    pub fn l2cap(address: u64, psm: u16) -> io::Result<Socket> {
        linux::l2cap(bdaddr(address), psm).map(|fd| Socket { fd })
    }

    /// Opens an RFCOMM channel, 1 to 30.
    #[cfg(target_os = "linux")]
    pub fn rfcomm(address: u64, channel: u8) -> io::Result<Socket> {
        linux::rfcomm(bdaddr(address), channel).map(|fd| Socket { fd })
    }

    /// Sends one packet (L2CAP) or a run of bytes (RFCOMM).
    #[cfg(target_os = "linux")]
    pub fn send(&self, data: &[u8]) -> io::Result<()> {
        linux::send(&self.fd, data)
    }

    /// Waits for the next packet or bytes; 0 when the channel closed.
    #[cfg(target_os = "linux")]
    pub fn recv(&self, buffer: &mut [u8]) -> io::Result<usize> {
        linux::recv(&self.fd, buffer)
    }

    /// Closes the channel, which also ends a `recv` waiting on it.
    #[cfg(target_os = "linux")]
    pub fn shutdown(&self) {
        linux::shutdown(&self.fd);
    }

    #[cfg(not(target_os = "linux"))]
    pub fn l2cap(_address: u64, _psm: u16) -> io::Result<Socket> {
        Err(unsupported())
    }

    #[cfg(not(target_os = "linux"))]
    pub fn rfcomm(_address: u64, _channel: u8) -> io::Result<Socket> {
        Err(unsupported())
    }

    #[cfg(not(target_os = "linux"))]
    pub fn send(&self, _data: &[u8]) -> io::Result<()> {
        Err(unsupported())
    }

    #[cfg(not(target_os = "linux"))]
    pub fn recv(&self, _buffer: &mut [u8]) -> io::Result<usize> {
        Err(unsupported())
    }

    #[cfg(not(target_os = "linux"))]
    pub fn shutdown(&self) {}
}

#[cfg(not(target_os = "linux"))]
fn unsupported() -> io::Error {
    io::Error::new(
        io::ErrorKind::Unsupported,
        "Bluetooth channels are opened on Linux only",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_go_least_significant_byte_first() {
        assert_eq!(
            bdaddr(0x80AF_1919_B477),
            [0x77, 0xB4, 0x19, 0x19, 0xAF, 0x80]
        );
    }
}
