//! Finds the addresses other PCs can reach this machine on: on the LAN,
//! or over a private VPN such as Tailscale or ZeroTier.

use std::net::{IpAddr, Ipv4Addr};

use serde::Serialize;

#[derive(Serialize, Clone)]
pub struct Address {
    pub ip: String,
    pub interface: String,
    /// The VPN the address belongs to ("Tailscale", "ZeroTier"), or `None`
    /// for an address on the LAN.
    pub vpn: Option<&'static str>,
}

/// The machine's IPv4 addresses, best guess first: the address of the
/// interface with the default route, then other private addresses, then
/// VPN addresses. Loopback and Docker's own bridges are left out.
pub fn lan_addresses() -> Vec<Address> {
    let primary = match local_ip_address::local_ip() {
        Ok(IpAddr::V4(ip)) => Some(ip),
        _ => None,
    };

    let mut found: Vec<(Ipv4Addr, String, Option<&'static str>)> = local_ip_address::list_afinet_netifas()
        .unwrap_or_default()
        .into_iter()
        .filter_map(|(name, ip)| match ip {
            IpAddr::V4(v4) if !v4.is_loopback() && !v4.is_link_local() && !is_virtual(&name) => {
                let vpn = vpn_name(&name, v4);
                Some((v4, name, vpn))
            }
            _ => None,
        })
        .collect();
    found.sort_by_key(|(ip, _, vpn)| (vpn.is_some(), Some(*ip) != primary, !ip.is_private()));
    found.dedup_by_key(|(ip, _, _)| *ip);

    found
        .into_iter()
        .map(|(ip, interface, vpn)| Address { ip: ip.to_string(), interface, vpn })
        .collect()
}

/// The address *Detect automatically* picks: never a VPN address, even when
/// the VPN carries the default route (a Tailscale exit node, say), since
/// players at home couldn't reach it without joining the VPN.
pub fn best_lan_ip() -> Option<String> {
    lan_addresses().into_iter().find(|a| a.vpn.is_none()).map(|a| a.ip)
}

/// Checks an address typed into the picker. rAthena writes it into its
/// configs and hands it to every client, so it has to be a plain IPv4
/// address another machine could connect to.
pub fn check_address(value: &str) -> Result<(), String> {
    let ip: Ipv4Addr = value
        .trim()
        .parse()
        .map_err(|_| format!("\"{}\" isn't an IPv4 address, such as 192.168.1.20 or 100.101.102.103.", value.trim()))?;
    if ip.is_unspecified() || ip.is_broadcast() || ip.is_multicast() {
        return Err(format!("{ip} can't be connected to; type this computer's address."));
    }
    Ok(())
}

fn is_virtual(interface: &str) -> bool {
    ["docker", "br-", "veth", "virbr", "lxcbr", "cni", "flannel"]
        .iter()
        .any(|p| interface.starts_with(p))
}

/// Tailscale's interface is `tailscale0` and its addresses are in the
/// 100.64.0.0/10 range it uses (as does Headscale); ZeroTier's interfaces
/// are named `zt` and a network id.
fn vpn_name(interface: &str, ip: Ipv4Addr) -> Option<&'static str> {
    let [a, b, ..] = ip.octets();
    if interface.starts_with("tailscale") || (a == 100 && (64..128).contains(&b)) {
        Some("Tailscale")
    } else if interface.starts_with("zt") {
        Some("ZeroTier")
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vpns() {
        let ip = |s: &str| s.parse().unwrap();
        assert_eq!(vpn_name("tailscale0", ip("100.101.102.103")), Some("Tailscale"));
        assert_eq!(vpn_name("tun0", ip("100.64.0.1")), Some("Tailscale"));
        assert_eq!(vpn_name("ztabcdef12", ip("10.147.17.5")), Some("ZeroTier"));
        assert_eq!(vpn_name("eth0", ip("192.168.1.20")), None);
        assert_eq!(vpn_name("wlan0", ip("100.128.0.1")), None);
    }

    #[test]
    fn typed_addresses() {
        assert!(check_address("192.168.1.20").is_ok());
        assert!(check_address(" 100.101.102.103 ").is_ok());
        assert!(check_address("127.0.0.1").is_ok());
        assert!(check_address("0.0.0.0").is_err());
        assert!(check_address("255.255.255.255").is_err());
        assert!(check_address("224.0.0.1").is_err());
        assert!(check_address("192.168.1").is_err());
        assert!(check_address("myserver.ts.net").is_err());
        assert!(check_address("1.2.3.4\nchar_ip: x").is_err());
    }
}
