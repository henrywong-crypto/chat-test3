# firecracker-client

HTTP client for the Firecracker microVM management API over a Unix domain socket.

## Responsibilities

- Send REST API requests to a running Firecracker process via its Unix socket
- Serialize request bodies to JSON

## API

```
set_boot_source(socket_path: &Path, boot_source: &BootSource) -> Result<()>
set_machine_config(socket_path: &Path, machine_config: &MachineConfig) -> Result<()>
set_drive(socket_path: &Path, drive: &Drive) -> Result<()>
set_network_interface(socket_path: &Path, iface: &NetworkInterface) -> Result<()>
set_mmds_config(socket_path: &Path, config: &MmdsConfig) -> Result<()>
put_mmds(socket_path: &Path, metadata: &serde_json::Value) -> Result<()>
start_instance(socket_path: &Path) -> Result<()>
```
