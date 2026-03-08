#!/usr/bin/env bash
set -euo pipefail

# Download
wget "https://s3.amazonaws.com/spec.ccfc.min/firecracker-ci/v1.14/x86_64/vmlinux-6.1.155"
wget -O ubuntu-24.04.squashfs.upstream "https://s3.amazonaws.com/spec.ccfc.min/firecracker-ci/v1.14/x86_64/ubuntu-24.04.squashfs"

# Unpack and set ownership
unsquashfs ubuntu-24.04.squashfs.upstream
sudo chown -R root:root squashfs-root
sudo chown -R 1000:1000 squashfs-root/home/ubuntu

# SSH key
ssh-keygen -f id_rsa -N ""
mkdir -p squashfs-root/home/ubuntu/.ssh
cp -v id_rsa.pub squashfs-root/home/ubuntu/.ssh/authorized_keys
sudo chmod 700 squashfs-root/home/ubuntu/.ssh
sudo chmod 600 squashfs-root/home/ubuntu/.ssh/authorized_keys
echo "nameserver 1.1.1.1" | sudo tee squashfs-root/etc/resolv.conf > /dev/null

# Install Node.js and Claude Code
sudo chmod 1777 squashfs-root/tmp
sudo mkdir -p squashfs-root/var/cache/apt/archives/partial
sudo mkdir -p squashfs-root/var/log/apt
sudo mount --bind /proc squashfs-root/proc
sudo mount --bind /sys  squashfs-root/sys
sudo mount --bind /dev  squashfs-root/dev
sudo chroot squashfs-root bash -c "
  apt-get update -qq &&
  apt-get install -y -qq nodejs npm &&
  npm install -g @anthropic-ai/claude-code
"
sudo umount squashfs-root/dev
sudo umount squashfs-root/sys
sudo umount squashfs-root/proc

# Claude Code settings
mkdir -p squashfs-root/home/ubuntu/.claude
cat > squashfs-root/home/ubuntu/.claude/settings.json << 'EOF'
{
  "$schema": "https://json.schemastore.org/claude-code-settings.json",
  "env": {
    "ANTHROPIC_DEFAULT_HAIKU_MODEL": "us.anthropic.claude-haiku-4-5-20251001-v1:0",
    "ANTHROPIC_DEFAULT_OPUS_MODEL": "us.anthropic.claude-opus-4-6-v1",
    "ANTHROPIC_DEFAULT_SONNET_MODEL": "us.anthropic.claude-sonnet-4-6",
    "CLAUDE_CODE_USE_BEDROCK": "1"
  }
}
EOF

# Build ext4 image
mv -v id_rsa ubuntu-24.04.id_rsa
truncate -s 10G ubuntu-24.04.ext4
sudo mkfs.ext4 -d squashfs-root -F ubuntu-24.04.ext4
sudo rm -rf squashfs-root

# Install to /var/lib/fc
sudo mkdir -p /var/lib/fc
sudo mv vmlinux-6.1.155 /var/lib/fc/vmlinux
sudo mv ubuntu-24.04.ext4 /var/lib/fc/ubuntu-24.04.ext4
sudo mv ubuntu-24.04.id_rsa /var/lib/fc/ubuntu-24.04.id_rsa

# Verify
echo
echo "The following files were installed:"
[ -f /var/lib/fc/vmlinux ]             && echo "Kernel:  /var/lib/fc/vmlinux"             || echo "ERROR: /var/lib/fc/vmlinux does not exist"
e2fsck -fn /var/lib/fc/ubuntu-24.04.ext4 &>/dev/null && echo "Rootfs:  /var/lib/fc/ubuntu-24.04.ext4" || echo "ERROR: /var/lib/fc/ubuntu-24.04.ext4 is not a valid ext4 fs"
[ -f /var/lib/fc/ubuntu-24.04.id_rsa ] && echo "SSH Key: /var/lib/fc/ubuntu-24.04.id_rsa" || echo "ERROR: /var/lib/fc/ubuntu-24.04.id_rsa does not exist"
