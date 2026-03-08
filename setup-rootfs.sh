#!/usr/bin/env bash
set -euo pipefail

# Download kernel
wget "https://s3.amazonaws.com/spec.ccfc.min/firecracker-ci/v1.14/x86_64/vmlinux-6.1.155"

# Download rootfs
wget -O ubuntu-24.04.squashfs.upstream "https://s3.amazonaws.com/spec.ccfc.min/firecracker-ci/v1.14/x86_64/ubuntu-24.04.squashfs"

# Unpack and patch rootfs
sudo unsquashfs ubuntu-24.04.squashfs.upstream
ssh-keygen -f id_rsa -N ""
mkdir -p squashfs-root/root/.ssh
cp -v id_rsa.pub squashfs-root/root/.ssh/authorized_keys
echo "nameserver 8.8.8.8" > squashfs-root/etc/resolv.conf
mv -v id_rsa ./ubuntu-24.04.id_rsa

# Build ext4 image
sudo chown -R root:root squashfs-root
truncate -s 1G ubuntu-24.04.ext4
sudo mkfs.ext4 -d squashfs-root -F ubuntu-24.04.ext4
sudo rm -rf squashfs-root

# Verify
echo
echo "The following files were downloaded and set up:"
[ -f vmlinux-6.1.155 ]    && echo "Kernel:  vmlinux-6.1.155"    || echo "ERROR: vmlinux-6.1.155 does not exist"
e2fsck -fn ubuntu-24.04.ext4 &>/dev/null && echo "Rootfs:  ubuntu-24.04.ext4" || echo "ERROR: ubuntu-24.04.ext4 is not a valid ext4 fs"
[ -f ubuntu-24.04.id_rsa ] && echo "SSH Key: ubuntu-24.04.id_rsa" || echo "ERROR: ubuntu-24.04.id_rsa does not exist"
