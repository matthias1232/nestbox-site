#!/usr/bin/env bash
# Prepare the Proxmox template Nestbox clones for rented nodes (ADR 0050). Run it ON the Proxmox host.
#
#   make-proxmox-template.sh <vmid> <storage> <image> <temp-ip/prefix> <gateway> <ssh-public-key-file>
#
# Example:
#   make-proxmox-template.sh 9000 local-btrfs /var/lib/vz/template/iso/noble-server-cloudimg-amd64.img \
#       192.168.55.52/24 192.168.55.1 ~/.ssh/id_ed25519.pub
#
# It imports an Ubuntu cloud image, boots it once on a temporary address, installs and enables the
# QEMU guest agent (the control plane runs the join command through it), resets cloud-init, shuts
# down and turns the VM into a template. The clone gets its own address from the account's pool.
set -euo pipefail

vmid="$1"; storage="$2"; image="$3"; ip="$4"; gateway="$5"; key="$6"

qm status "$vmid" >/dev/null 2>&1 && { echo "VM $vmid exists; pick another id." >&2; exit 1; }
qm create "$vmid" --name nestbox-node-template --memory 2048 --cores 2 --cpu host \
  --net0 virtio,bridge=vmbr0 --scsihw virtio-scsi-pci --ostype l26 --agent enabled=1
qm importdisk "$vmid" "$image" "$storage" >/dev/null
qm set "$vmid" --scsi0 "$storage:$vmid/vm-$vmid-disk-0.raw" --boot order=scsi0 \
  --ide2 "$storage:cloudinit" --serial0 socket --vga serial0
qm resize "$vmid" scsi0 24G >/dev/null
qm set "$vmid" --ciuser ubuntu --sshkeys "$key" --ipconfig0 "ip=$ip,gw=$gateway" --nameserver 1.1.1.1
qm start "$vmid"

address="${ip%/*}"
echo "waiting for $address ..."
for _ in $(seq 1 60); do
  ssh -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null -o ConnectTimeout=3 -o BatchMode=yes "ubuntu@$address" true 2>/dev/null && break
  sleep 5
done
ssh -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null "ubuntu@$address" 'sudo bash -s' <<'GUEST'
set -e
cloud-init status --wait >/dev/null 2>&1 || true
export DEBIAN_FRONTEND=noninteractive
apt-get update -qq
apt-get install -y -qq qemu-guest-agent wireguard-tools python3 curl ca-certificates openssl >/dev/null
systemctl enable qemu-guest-agent
cloud-init clean --logs
rm -f /etc/ssh/ssh_host_* /etc/machine-id
touch /etc/machine-id
GUEST
ssh -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null "ubuntu@$address" 'sudo poweroff' || true
for _ in $(seq 1 30); do [ "$(qm status "$vmid" | awk '{print $2}')" = stopped ] && break; sleep 3; done
qm set "$vmid" --delete sshkeys,ipconfig0 >/dev/null 2>&1 || true
qm template "$vmid"
echo "Template $vmid is ready. Use template_vmid=$vmid in the Proxmox account."
