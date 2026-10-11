//! The Windows sources of the machine proxy: WinHTTP's default proxy configuration, read once, and this machine's
//! interface addresses, listed once (design §6).

use super::{Access, AddressSource, Configuration, MachineProxy, ProxySource};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use windows_sys::Win32::Foundation::{ERROR_BUFFER_OVERFLOW, ERROR_SUCCESS, GlobalFree};
use windows_sys::Win32::NetworkManagement::IpHelper::{
    GAA_FLAG_SKIP_ANYCAST, GAA_FLAG_SKIP_DNS_SERVER, GAA_FLAG_SKIP_MULTICAST, GetAdaptersAddresses,
    IP_ADAPTER_ADDRESSES_LH,
};
use windows_sys::Win32::Networking::WinHttp::{
    WINHTTP_ACCESS_TYPE_NAMED_PROXY, WINHTTP_ACCESS_TYPE_NO_PROXY, WINHTTP_PROXY_INFO,
    WinHttpGetDefaultProxyConfiguration,
};
use windows_sys::Win32::Networking::WinSock::{
    AF_INET, AF_INET6, AF_UNSPEC, SOCKADDR, SOCKADDR_IN, SOCKADDR_IN6,
};

/// The longest wide string read from WinHTTP, in UTF-16 units. A longer one is unreadable.
const MAX_UNITS: usize = 1 << 16;

/// The machine proxy as WinHTTP and the interface list state it now.
pub(crate) fn capture_machine() -> MachineProxy {
    MachineProxy::capture(&WinHttp, &Interfaces)
}

/// `WinHttpGetDefaultProxyConfiguration`.
struct WinHttp;

impl ProxySource for WinHttp {
    fn read(&self) -> Configuration {
        // SAFETY: an initialized exclusive output. Every returned allocation, including those of a failed call, is
        // disposed below on every exit.
        let mut info: WINHTTP_PROXY_INFO = unsafe { std::mem::zeroed() };
        let ok = unsafe { WinHttpGetDefaultProxyConfiguration(&mut info) };
        let proxy = wide(info.lpszProxy);
        let bypass = wide(info.lpszProxyBypass);
        for pointer in [info.lpszProxy, info.lpszProxyBypass] {
            if !pointer.is_null() {
                // SAFETY: WinHTTP returns storage allocated for GlobalFree.
                unsafe {
                    GlobalFree(pointer.cast());
                }
            }
        }
        let (Ok(proxy), Ok(bypass)) = (proxy, bypass) else {
            return unreadable();
        };
        if ok == 0 {
            return unreadable();
        }
        let access = match info.dwAccessType {
            WINHTTP_ACCESS_TYPE_NO_PROXY => Access::Direct,
            WINHTTP_ACCESS_TYPE_NAMED_PROXY => Access::Named,
            _ => Access::Automatic,
        };
        Configuration {
            access,
            proxy,
            bypass,
        }
    }
}

fn unreadable() -> Configuration {
    Configuration {
        access: Access::Unreadable,
        ..Configuration::default()
    }
}

/// A NUL-terminated wide string as text; a null pointer is no text, and a string that is not UTF-16 or has no end
/// within `MAX_UNITS` is unreadable.
fn wide(pointer: *const u16) -> Result<Option<String>, ()> {
    if pointer.is_null() {
        return Ok(None);
    }
    let mut units = Vec::new();
    for index in 0..=MAX_UNITS {
        // SAFETY: WinHTTP returned a NUL-terminated string and the scan stops at the first NUL.
        let unit = unsafe { *pointer.add(index) };
        if unit == 0 {
            return String::from_utf16(&units).map(Some).map_err(|_| ());
        }
        units.push(unit);
    }
    Err(())
}

/// `GetAdaptersAddresses`: every unicast address of every interface. A failure lists none, which sends an own-address
/// origin to the proxy and so fails toward the proxy.
struct Interfaces;

impl AddressSource for Interfaces {
    fn addresses(&self) -> Vec<IpAddr> {
        let flags = GAA_FLAG_SKIP_ANYCAST | GAA_FLAG_SKIP_MULTICAST | GAA_FLAG_SKIP_DNS_SERVER;
        let mut size: u32 = 16 * 1024;
        for _ in 0..4 {
            // 8-byte aligned storage for the adapters' linked list.
            let mut buffer = vec![0u64; (size as usize).div_ceil(8)];
            let first = buffer.as_mut_ptr().cast::<IP_ADAPTER_ADDRESSES_LH>();
            // SAFETY: `first` points at `size` writable bytes, and `size` is updated when they do not suffice.
            let status = unsafe {
                GetAdaptersAddresses(
                    u32::from(AF_UNSPEC),
                    flags,
                    std::ptr::null(),
                    first,
                    &mut size,
                )
            };
            if status == ERROR_SUCCESS {
                // SAFETY: a successful call filled `buffer` with a list that lives as long as `buffer` does.
                return unsafe { collect(first) };
            }
            if status != ERROR_BUFFER_OVERFLOW {
                break;
            }
        }
        Vec::new()
    }
}

/// The unicast addresses of the adapter list starting at `adapter`.
///
/// # Safety
/// `adapter` is null or the head of a list `GetAdaptersAddresses` filled, still alive.
unsafe fn collect(mut adapter: *const IP_ADAPTER_ADDRESSES_LH) -> Vec<IpAddr> {
    let mut found = Vec::new();
    while !adapter.is_null() {
        // SAFETY: the list is valid for the caller's contract.
        let current = unsafe { &*adapter };
        let mut unicast = current.FirstUnicastAddress.cast_const();
        while !unicast.is_null() {
            // SAFETY: as above.
            let entry = unsafe { &*unicast };
            if let Some(address) = unsafe { socket_address(entry.Address.lpSockaddr) } {
                found.push(address);
            }
            unicast = entry.Next.cast_const();
        }
        adapter = current.Next.cast_const();
    }
    found
}

/// The IP address a socket address holds, if it is an IPv4 or IPv6 one.
///
/// # Safety
/// `address` is null or points at a socket address of the family it names.
unsafe fn socket_address(address: *const SOCKADDR) -> Option<IpAddr> {
    if address.is_null() {
        return None;
    }
    // SAFETY: the family field is the first member of every socket address.
    let family = unsafe { (*address).sa_family };
    if family == AF_INET {
        // SAFETY: the family says this is a SOCKADDR_IN.
        let v4 = unsafe { &*address.cast::<SOCKADDR_IN>() };
        // SAFETY: every member of the union is plain data.
        let raw = unsafe { v4.sin_addr.S_un.S_addr };
        Some(IpAddr::V4(Ipv4Addr::from(raw.to_ne_bytes())))
    } else if family == AF_INET6 {
        // SAFETY: the family says this is a SOCKADDR_IN6.
        let v6 = unsafe { &*address.cast::<SOCKADDR_IN6>() };
        // SAFETY: every member of the union is plain data.
        let bytes = unsafe { v6.sin6_addr.u.Byte };
        Some(IpAddr::V6(Ipv6Addr::from(bytes)))
    } else {
        None
    }
}
