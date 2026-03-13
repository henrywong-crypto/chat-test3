use std::path::PathBuf;

pub struct VmBuildConfig {
    pub kernel_path: PathBuf,
    pub net_helper_path: PathBuf,
    pub vcpu_count: u8,
    pub mem_size_mib: u32,
    pub jailer_path: PathBuf,
    pub firecracker_path: PathBuf,
    pub jailer_uid: u32,
    pub jailer_gid: u32,
    pub jailer_chroot_base: PathBuf,
}
