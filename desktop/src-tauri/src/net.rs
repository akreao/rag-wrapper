//! Finds the addresses other PCs on the LAN can reach this machine on.

use std::net::{IpAddr, Ipv4Addr};

use serde::Serialize;

#[derive(Serialize, Clone)]
pub struct Address {
    pub ip: String,
    pub interface: String,
}

/// The machine's IPv4 addresses, best guess first: the address of the
/// interface with the default route, then other private addresses.
/// Loopback and Docker's own bridges are left out.
pub fn lan_addresses() -> Vec<Address> {
    let primary = match local_ip_address::local_ip() {
        Ok(IpAddr::V4(ip)) => Some(ip),
        _ => None,
    };

    let mut found: Vec<(Ipv4Addr, String)> = local_ip_address::list_afinet_netifas()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|(name, ip)| match ip {
            IpAddr::V4(v4) if !v4.is_loopback() && !v4.is_link_local() && !is_virtual(&name) => {
                Some((v4, name))
            }
            _ => None,
        })
        .collect();
    found.sort_by_key(|(ip, _)| (Some(*ip) != primary, !ip.is_private()));
    found.dedup_by_key(|(ip, _)| *ip);

    found
        .into_iter()
        .map(|(ip, interface)| Address { ip: ip.to_string(), interface })
        .collect()
}

pub fn best_lan_ip() -> Option<String> {
    lan_addresses().into_iter().next().map(|a| a.ip)
}

fn is_virtual(interface: &str) -> bool {
    ["docker", "br-", "veth", "virbr", "lxcbr", "cni", "flannel"]
        .iter()
        .any(|p| interface.starts_with(p))
}
