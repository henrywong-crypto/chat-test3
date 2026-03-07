# firecracker-client

- HTTP client over Unix domain socket
- Firecracker API types: `MachineConfig`, `BootSource`, `Drive`, `InstanceAction`

## Functions

```
set_machine_config(socket_path: &Path, machine_config: &MachineConfig) -> Result<()>
set_boot_source(socket_path: &Path, boot_source: &BootSource) -> Result<()>
set_drive(socket_path: &Path, drive: &Drive) -> Result<()>
start_instance(socket_path: &Path) -> Result<()>
```
