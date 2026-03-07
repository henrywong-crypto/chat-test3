# firecracker-manager

- Allocates socket path and tap interface per VM
- Creates PTY pair via `terminal-bridge`, spawns `firecracker` with PTY slave as stdin/stdout
- Configures VM via `firecracker-client`, then starts it
- On server startup: reconciles persisted VMs (re-attach live, mark dead)

## Functions

```
create_vm(vm_config: &VmConfig) -> Result<Vm>
kill_vm(vm_id: &str) -> Result<()>
reconcile_vms(sessions: &[Session]) -> Result<()>
```
