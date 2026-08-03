use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::sync::OnceLock;

pub fn addresses() -> &'static Vec<IpAddr> {
    static ADDRESSES: OnceLock<Vec<IpAddr>> = OnceLock::new();
    ADDRESSES.get_or_init(|| {
        let mut addresses: Vec<IpAddr> = Vec::new();

        if let Ok(networks) = local_ip_address::list_afinet_netifas() {
            for (_, address) in networks {
                match address {
                    IpAddr::V4(ip) => {
                        let parts = ip.octets();
                        if !(parts[0] == 10 && parts[1] == 144 && parts[2] == 144)
                            && ip != Ipv4Addr::LOCALHOST
                            && ip != Ipv4Addr::UNSPECIFIED
                        {
                            addresses.push(IpAddr::V4(ip));
                        }
                    }
                    IpAddr::V6(ip) => {
                        if ip != Ipv6Addr::LOCALHOST && ip != Ipv6Addr::UNSPECIFIED {
                            addresses.push(IpAddr::V6(ip));
                        }
                    }
                }
            }
        }

        addresses.push(IpAddr::V4(Ipv4Addr::UNSPECIFIED));
        addresses.push(IpAddr::V6(Ipv6Addr::UNSPECIFIED));

        addresses.sort_by(|ip1, ip2| ip2.cmp(ip1));
        addresses
    })
}
