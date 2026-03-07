use std::process::Command;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    std::process::exit(run(&args));
}

fn run(args: &[String]) -> i32 {
    // Raise CAP_NET_ADMIN into the ambient set so child processes (ip, iptables)
    // inherit it through exec without needing their own file capabilities.
    // Requires net-helper to be deployed with cap_net_admin=eip (not just =ep).
    if let Err(e) = raise_ambient_net_admin() {
        eprintln!("failed to raise ambient cap_net_admin: {e}");
        eprintln!("hint: deploy with 'sudo setcap cap_net_admin=eip /usr/local/bin/net-helper'");
        return 2;
    }

    match args.get(1).map(|s| s.as_str()) {
        Some("tap-create") => {
            if args.len() != 4 {
                eprintln!("usage: net-helper tap-create <tap-name> <cidr>");
                return 1;
            }
            let tap_name = &args[2];
            let cidr = &args[3];
            if let Err(e) = validate_tap_name(tap_name) {
                eprintln!("invalid tap name: {e}");
                return 1;
            }
            if let Err(e) = validate_cidr(cidr) {
                eprintln!("invalid cidr: {e}");
                return 1;
            }
            cmd_tap_create(tap_name, cidr)
        }
        Some("tap-delete") => {
            if args.len() != 3 {
                eprintln!("usage: net-helper tap-delete <tap-name>");
                return 1;
            }
            let tap_name = &args[2];
            if let Err(e) = validate_tap_name(tap_name) {
                eprintln!("invalid tap name: {e}");
                return 1;
            }
            cmd_tap_delete(tap_name)
        }
        Some("setup-nat") => {
            if args.len() != 3 {
                eprintln!("usage: net-helper setup-nat <host-iface>");
                return 1;
            }
            let iface = &args[2];
            if let Err(e) = validate_iface_name(iface) {
                eprintln!("invalid interface name: {e}");
                return 1;
            }
            cmd_setup_nat(iface)
        }
        _ => {
            eprintln!("usage: net-helper <tap-create|tap-delete|setup-nat> ...");
            1
        }
    }
}

fn validate_tap_name(name: &str) -> Result<(), &'static str> {
    let digits = name.strip_prefix("tap").ok_or("must start with 'tap'")?;
    if digits.is_empty() || digits.len() > 3 {
        return Err("digits part must be 1-3 characters");
    }
    if !digits.chars().all(|c| c.is_ascii_digit()) {
        return Err("digits part must be numeric");
    }
    if digits.len() > 1 && digits.starts_with('0') {
        return Err("no leading zeros in tap index");
    }
    let n: u32 = digits.parse().map_err(|_| "invalid number")?;
    if n > 253 {
        return Err("tap index must be 0-253");
    }
    Ok(())
}

fn validate_cidr(cidr: &str) -> Result<(), &'static str> {
    let (ip_str, prefix_str) = cidr.split_once('/').ok_or("missing '/' in cidr")?;
    if prefix_str.is_empty() {
        return Err("missing prefix length");
    }
    if !prefix_str.chars().all(|c| c.is_ascii_digit()) {
        return Err("prefix length must be numeric");
    }
    let prefix: u8 = prefix_str.parse().map_err(|_| "invalid prefix length")?;
    if prefix > 32 {
        return Err("prefix length must be 0-32");
    }
    let octets: Vec<&str> = ip_str.split('.').collect();
    if octets.len() != 4 {
        return Err("IPv4 address must have 4 octets");
    }
    for octet in &octets {
        if octet.is_empty() {
            return Err("empty octet");
        }
        if !octet.chars().all(|c| c.is_ascii_digit()) {
            return Err("octets must be numeric");
        }
        if octet.len() > 1 && octet.starts_with('0') {
            return Err("no leading zeros in octets");
        }
        let n: u16 = octet.parse().map_err(|_| "invalid octet value")?;
        if n > 255 {
            return Err("octet value must be 0-255");
        }
    }
    Ok(())
}

fn validate_iface_name(name: &str) -> Result<(), &'static str> {
    if name.is_empty() || name.len() > 15 {
        return Err("interface name must be 1-15 characters");
    }
    if name == "." || name == ".." {
        return Err("interface name must not be '.' or '..'");
    }
    if !name.chars().all(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | '@' | '.')) {
        return Err("interface name contains invalid characters");
    }
    Ok(())
}

fn run_cmd(prog: &str, args: &[&str]) -> i32 {
    match Command::new(prog).args(args).status() {
        Ok(s) if s.success() => 0,
        Ok(s) => {
            eprintln!("{prog} {:?} failed: exit {}", args, s.code().unwrap_or(-1));
            2
        }
        Err(e) => {
            eprintln!("failed to run {prog}: {e}");
            2
        }
    }
}

fn cmd_tap_create(tap_name: &str, cidr: &str) -> i32 {
    // Delete if stale (best-effort)
    let _ = Command::new("ip").args(["link", "del", tap_name]).status();
    let r = run_cmd("ip", &["tuntap", "add", "dev", tap_name, "mode", "tap"]);
    if r != 0 {
        return r;
    }
    let r = run_cmd("ip", &["addr", "add", cidr, "dev", tap_name]);
    if r != 0 {
        return r;
    }
    run_cmd("ip", &["link", "set", "dev", tap_name, "up"])
}

fn cmd_tap_delete(tap_name: &str) -> i32 {
    run_cmd("ip", &["link", "del", tap_name])
}

fn cmd_setup_nat(iface: &str) -> i32 {
    if let Err(e) = std::fs::write("/proc/sys/net/ipv4/ip_forward", "1") {
        eprintln!("failed to enable ip_forward: {e}");
        return 2;
    }
    let r = run_cmd("iptables", &["-P", "FORWARD", "ACCEPT"]);
    if r != 0 {
        return r;
    }
    // Best-effort delete to avoid duplicates on restart
    let _ = Command::new("iptables")
        .args(["-t", "nat", "-D", "POSTROUTING", "-o", iface, "-j", "MASQUERADE"])
        .stderr(std::process::Stdio::null())
        .status();
    run_cmd("iptables", &["-t", "nat", "-A", "POSTROUTING", "-o", iface, "-j", "MASQUERADE"])
}

fn raise_ambient_net_admin() -> std::io::Result<()> {
    #[cfg(target_os = "linux")]
    unsafe {
        const CAP_NET_ADMIN: u32 = 12;
        const CAP_V3: u32 = 0x2008_0522; // _LINUX_CAPABILITY_VERSION_3
        const PR_CAP_AMBIENT: libc::c_int = 47;
        const PR_CAP_AMBIENT_RAISE: libc::c_ulong = 2;

        #[repr(C)]
        struct CapHdr { version: u32, pid: i32 }
        #[repr(C)]
        #[derive(Clone, Copy, Default)]
        struct CapData { effective: u32, permitted: u32, inheritable: u32 }

        let mut hdr = CapHdr { version: CAP_V3, pid: 0 };
        let mut data = [CapData::default(); 2];

        // capget — read current sets
        if libc::syscall(libc::SYS_capget, &mut hdr as *mut CapHdr, data.as_mut_ptr()) < 0 {
            return Err(std::io::Error::last_os_error());
        }
        // Promote CAP_NET_ADMIN into inheritable (allowed since it's in permitted)
        data[0].inheritable |= 1 << CAP_NET_ADMIN;
        // capset — write back
        if libc::syscall(libc::SYS_capset, &hdr as *const CapHdr, data.as_ptr()) < 0 {
            return Err(std::io::Error::last_os_error());
        }
        // Raise to ambient so exec'd children inherit it
        if libc::prctl(PR_CAP_AMBIENT, PR_CAP_AMBIENT_RAISE, CAP_NET_ADMIN as libc::c_ulong, 0, 0) < 0 {
            return Err(std::io::Error::last_os_error());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tap_name_valid() {
        assert!(validate_tap_name("tap0").is_ok());
        assert!(validate_tap_name("tap1").is_ok());
        assert!(validate_tap_name("tap9").is_ok());
        assert!(validate_tap_name("tap10").is_ok());
        assert!(validate_tap_name("tap99").is_ok());
        assert!(validate_tap_name("tap100").is_ok());
        assert!(validate_tap_name("tap253").is_ok());
    }

    #[test]
    fn tap_name_invalid() {
        assert!(validate_tap_name("").is_err());
        assert!(validate_tap_name("tap").is_err());
        assert!(validate_tap_name("eth0").is_err());
        assert!(validate_tap_name("tap00").is_err());
        assert!(validate_tap_name("tap01").is_err());
        assert!(validate_tap_name("tap254").is_err());
        assert!(validate_tap_name("tap999").is_err());
        assert!(validate_tap_name("tap1234").is_err());
        assert!(validate_tap_name("tapx").is_err());
        assert!(validate_tap_name("tap-1").is_err());
    }

    #[test]
    fn cidr_valid() {
        assert!(validate_cidr("0.0.0.0/0").is_ok());
        assert!(validate_cidr("172.16.0.1/30").is_ok());
        assert!(validate_cidr("192.168.1.1/24").is_ok());
        assert!(validate_cidr("10.0.0.1/8").is_ok());
        assert!(validate_cidr("255.255.255.255/32").is_ok());
    }

    #[test]
    fn cidr_invalid() {
        assert!(validate_cidr("172.016.0.1/30").is_err());
        assert!(validate_cidr("172.16.0.1").is_err());
        assert!(validate_cidr("172.16.0.1/33").is_err());
        assert!(validate_cidr("172.16.0/30").is_err());
        assert!(validate_cidr("172.16.0.256/30").is_err());
        assert!(validate_cidr("172.16.0.1/").is_err());
        assert!(validate_cidr("").is_err());
        assert!(validate_cidr("abc.def.ghi.jkl/24").is_err());
    }

    #[test]
    fn iface_name_valid() {
        assert!(validate_iface_name("eth0").is_ok());
        assert!(validate_iface_name("ens3").is_ok());
        assert!(validate_iface_name("wlan0").is_ok());
        assert!(validate_iface_name("lo").is_ok());
        assert!(validate_iface_name("docker0").is_ok());
        assert!(validate_iface_name("veth@if5").is_ok());
        assert!(validate_iface_name("a").is_ok());
        assert!(validate_iface_name("123456789012345").is_ok());
    }

    #[test]
    fn iface_name_invalid() {
        assert!(validate_iface_name("").is_err());
        assert!(validate_iface_name(".").is_err());
        assert!(validate_iface_name("..").is_err());
        assert!(validate_iface_name("1234567890123456").is_err());
        assert!(validate_iface_name("eth 0").is_err());
        assert!(validate_iface_name("eth/0").is_err());
    }
}
