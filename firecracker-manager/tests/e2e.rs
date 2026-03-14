use anyhow::{Context, Result};
use firecracker_manager::{JailerConfig, VmConfig, create_vm, setup_host_networking};
use nix::{sys::signal::kill, unistd::Pid};
use serde::Deserialize;
use std::path::PathBuf;
use uuid::Uuid;

#[derive(Deserialize)]
struct JailerTestConfig {
    jailer_path: PathBuf,
    firecracker_path: PathBuf,
    uid: u32,
    gid: u32,
    chroot_base: PathBuf,
}

#[derive(Deserialize)]
struct TestConfig {
    kernel_path: PathBuf,
    rootfs_path: PathBuf,
    net_helper_path: PathBuf,
    jailer: Option<JailerTestConfig>,
}

fn read_test_config() -> Result<TestConfig> {
    let config_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../config.toml");
    let content = std::fs::read_to_string(&config_path)
        .with_context(|| format!("could not read {}", config_path.display()))?;
    toml::from_str(&content).context("invalid config.toml")
}

fn build_test_vm_config(test_config: &TestConfig) -> VmConfig {
    VmConfig {
        id: Uuid::new_v4().to_string(),
        socket_dir: PathBuf::from("/tmp"),
        kernel_path: test_config.kernel_path.clone(),
        rootfs_path: test_config.rootfs_path.clone(),
        net_helper_path: test_config.net_helper_path.clone(),
        vcpu_count: 1,
        mem_size_mib: 512,
        boot_args: "reboot=k panic=1 quiet loglevel=3 selinux=0 8250.nr_uarts=0".to_string(),
        mmds_metadata: None,
        mmds_imds_compat: false,
        jailer: None,
    }
}

#[tokio::test]
#[ignore = "requires firecracker binary, kernel, rootfs, and KVM access"]
async fn test_vm_boots() -> Result<()> {
    let test_config = read_test_config()?;
    setup_host_networking(&test_config.net_helper_path).await;
    let vm_config = build_test_vm_config(&test_config);
    let vm_guard = create_vm(&vm_config).await?;
    assert!(
        kill(Pid::from_raw(vm_guard.pid as i32), None).is_ok(),
        "VM process is not running"
    );
    assert!(
        !vm_guard.guest_ip.is_empty(),
        "guest IP should not be empty"
    );
    // VmGuard drops here, killing the VM and cleaning up TAP/files
    Ok(())
}

#[tokio::test]
#[ignore = "requires jailer binary, firecracker binary, kernel, rootfs, KVM access, and root privileges"]
async fn test_vm_boots_with_jailer() -> Result<()> {
    let test_config = read_test_config()?;
    let jailer_test_config = test_config
        .jailer
        .as_ref()
        .context("[jailer] section required in config.toml for this test")?;

    setup_host_networking(&test_config.net_helper_path).await;

    let vm_id = Uuid::new_v4().to_string();
    let chroot_dir = jailer_test_config
        .chroot_base
        .join("firecracker")
        .join(&vm_id)
        .join("root");

    let vm_config = VmConfig {
        id: vm_id,
        socket_dir: PathBuf::from("/tmp"),
        kernel_path: test_config.kernel_path.clone(),
        rootfs_path: test_config.rootfs_path.clone(),
        net_helper_path: test_config.net_helper_path.clone(),
        vcpu_count: 1,
        mem_size_mib: 512,
        boot_args: "reboot=k panic=1 quiet loglevel=3 selinux=0 8250.nr_uarts=0".to_string(),
        mmds_metadata: None,
        mmds_imds_compat: false,
        jailer: Some(JailerConfig {
            jailer_path: jailer_test_config.jailer_path.clone(),
            firecracker_path: jailer_test_config.firecracker_path.clone(),
            uid: jailer_test_config.uid,
            gid: jailer_test_config.gid,
            chroot_base: jailer_test_config.chroot_base.clone(),
        }),
    };

    let vm_guard = create_vm(&vm_config).await?;

    assert!(
        kill(Pid::from_raw(vm_guard.pid as i32), None).is_ok(),
        "VM process is not running"
    );
    assert!(
        !vm_guard.guest_ip.is_empty(),
        "guest IP should not be empty"
    );
    assert!(
        chroot_dir.exists(),
        "chroot dir should exist while VM is running: {}",
        chroot_dir.display()
    );

    drop(vm_guard);

    assert!(
        !chroot_dir.exists(),
        "chroot dir should be removed after VM drops: {}",
        chroot_dir.display()
    );
    Ok(())
}
