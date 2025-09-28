use std::net::IpAddr;

use pnet::datalink::{self};

pub(crate) fn get_all_local_ip_addresses() -> Vec<IpAddr> {
    let mut ips = Vec::new();

    for interface in datalink::interfaces() {
        for ip in &interface.ips {
            ips.push(ip.ip());
        }
    }

    ips
}
