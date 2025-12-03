use std::{net::IpAddr, os::fd::AsFd, time::Duration};

use pnet::datalink::{self};

// get_all_local_ip_addresses ------------------------------------------

pub(crate) fn get_all_local_ip_addresses() -> Vec<IpAddr> {
    let mut ips = Vec::new();

    for interface in datalink::interfaces() {
        for ip in &interface.ips {
            ips.push(ip.ip());
        }
    }

    ips
}

pub(crate) fn setup_keepalive(
    stream: &tokio::net::TcpStream,
    interval: Duration,
) -> std::io::Result<()> {
    let interval_sec = interval.as_secs_f64().ceil() as i32;

    if interval_sec == 0 {
        return Ok(());
    }

    #[cfg(target_os = "linux")]
    unsafe {
        use std::os::fd::AsRawFd;

        if libc::setsockopt(
            stream.as_fd().as_raw_fd(),
            libc::IPPROTO_TCP,
            libc::TCP_KEEPIDLE,
            &interval_sec as *const _ as *const libc::c_void,
            size_of_val(&interval_sec) as libc::socklen_t,
        ) != 0
        {
            return std::io::Result::Err(std::io::Error::last_os_error());
        }

        if libc::setsockopt(
            stream.as_fd().as_raw_fd(),
            libc::IPPROTO_TCP,
            libc::TCP_KEEPINTVL,
            &interval_sec as *const _ as *const libc::c_void,
            size_of_val(&interval_sec) as libc::socklen_t,
        ) != 0
        {
            return std::io::Result::Err(std::io::Error::last_os_error());
        }

        Ok(())
    }

    #[cfg(target_os = "macos")]
    unsafe {
        use std::os::fd::AsRawFd;

        if libc::setsockopt(
            stream.as_fd().as_raw_fd(),
            libc::IPPROTO_TCP,
            libc::TCP_KEEPALIVE,
            &interval_sec as *const _ as *const _,
            std::mem::size_of_val(&interval_sec) as libc::socklen_t,
        ) != 0
        {
            return std::io::Result::Err(std::io::Error::last_os_error());
        }

        Ok(())
    }
}
