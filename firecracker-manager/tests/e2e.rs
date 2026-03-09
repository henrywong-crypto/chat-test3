use firecracker_manager::{create_vm, setup_host_networking, VmConfig};
use nix::{sys::signal::kill, unistd::Pid};
use serde::Deserialize;
use std::path::PathBuf;
use uuid::Uuid;

#[derive(Deserialize)]
struct TestConfig {
    kernel_path: PathBuf,
    rootfs_path: PathBuf,
}

fn read_test_config() -> TestConfig {
    let config_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../config.toml");
    let content = std::fs::read_to_string(&config_path)
        .unwrap_or_else(|_| panic!("could not read {}", config_path.display()));
    toml::from_str(&content)
        .unwrap_or_else(|e| panic!("invalid config.toml: {e}"))
}

fn build_test_vm_config(kernel_path: PathBuf, rootfs_path: PathBuf) -> VmConfig {
    VmConfig {
        id: Uuid::new_v4().to_string(),
        socket_dir: PathBuf::from("/tmp"),
        kernel_path,
        rootfs_path,
        vcpu_count: 1,
        mem_size_mib: 512,
        boot_args: "reboot=k panic=1 quiet loglevel=3 selinux=0".to_string(),
        mmds_metadata: None,
        mmds_imds_compat: false,
    }
}

#[tokio::test]
#[ignore = "requires firecracker binary, kernel, rootfs, and KVM access"]
async fn test_vm_boots() {
    let test_config = read_test_config();
    setup_host_networking().await;
    let vm_config = build_test_vm_config(test_config.kernel_path, test_config.rootfs_path);
    let vm = create_vm(&vm_config).await.expect("VM failed to start");
    let vm_guard = vm.into_guard();
    assert!(
        kill(Pid::from_raw(vm_guard.pid as i32), None).is_ok(),
        "VM process is not running"
    );
    assert!(!vm_guard.guest_ip.is_empty(), "guest IP should not be empty");
    // VmGuard drops here, killing the VM and cleaning up TAP/files
}
