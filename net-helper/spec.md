# net-helper

Privileged helper binary for TAP device management and NAT setup.

Must be deployed with `cap_net_admin=eip` so it can create network interfaces without running the main server as root. On startup it raises `CAP_NET_ADMIN` into the inheritable and ambient sets (via the `caps` crate) so spawned subprocesses (`ip`, `iptables`) inherit the capability.

## Commands

```
net-helper tap-create <tap-name> <cidr>   # create TAP, assign IP, bring up
net-helper tap-delete <tap-name>          # delete TAP interface
net-helper setup-nat <host-iface>         # enable ip_forward + iptables MASQUERADE
```

## Validation

- `tap-name`: must match `tap[0-9]{1,3}` with index 0–253, no leading zeros
- `cidr`: standard IPv4 CIDR notation validated via `ipnet::Ipv4Net`
- `host-iface`: 1–15 alphanumeric characters (plus `-`, `_`, `@`, `.`), not `.` or `..`

## Deployment

```bash
sudo setcap cap_net_admin=eip /usr/local/bin/net-helper
```
