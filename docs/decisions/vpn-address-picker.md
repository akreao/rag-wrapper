# VPN address picker

Roadmap feature 8, built in the desktop app: friends outside the home network play through a private VPN (Tailscale or ZeroTier) that the host and players join themselves. There's still no public internet exposure, so the LAN-only decisions (MD5 passwords, open registration) stand.

- **The app only detects VPNs.** It lists Tailscale addresses (the `tailscale0` interface, or any address in Tailscale's 100.64.0.0/10 range) and ZeroTier addresses (`zt*` interfaces) next to the LAN ones. It doesn't install, sign in to or manage the VPN.
- **"Detect automatically" never picks a VPN address**, even when the VPN carries the default route (a Tailscale exit node), so a home server stays reachable at home unless a VPN address is chosen on purpose.
- **A typed address is IPv4 only.** rAthena hands the address to every client and the portal writes it into `clientinfo.xml`, so the app accepts a plain IPv4 address (not 0.0.0.0, broadcast or multicast). Hostnames such as Tailscale's MagicDNS names aren't accepted.
- **One address for everyone, and the picker says so.** rAthena sends every player the same char and map address. With a VPN address chosen, players at home must join the VPN too; with a LAN address, friends on the VPN can't get past the server list. A note under the picker spells out whichever case applies.
- **A change while the server runs asks first, then restarts char, map and the portal.** Those are the services that use `PUBLIC_IP`; the login server and database keep running. When a rebuilt image is waiting, the change does the full `up` instead, so database upgrades run first. A stopped server just uses the new address at its next Start.
