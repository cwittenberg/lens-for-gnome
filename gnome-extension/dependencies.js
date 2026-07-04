import GLib from 'gi://GLib';
import { runtime } from './runtime.js';

export function checkDependencies() {
    let hasFfmpeg = GLib.find_program_in_path('ffmpeg') !== null;
    let hasGst = GLib.find_program_in_path('gst-inspect-1.0') !== null || GLib.find_program_in_path('gst-launch-1.0') !== null;

    return hasFfmpeg && hasGst;
}

export function checkDaemon() {
    return runtime.isDaemonInstalled();
}

export function getDaemonInstallCommand() {
    return "sudo snap install lens-for-gnome && sudo snap connect lens-for-gnome:removable-media";
}

export function getDistributionInstructions() {
    let instructions = {
        debian: "sudo apt install ffmpeg gir1.2-gstreamer-1.0 gir1.2-cogl-1.0 gstreamer1.0-plugins-base gstreamer1.0-plugins-good gstreamer1.0-plugins-bad gstreamer1.0-plugins-ugly gstreamer1.0-libav",
        fedora: "sudo dnf install ffmpeg gstreamer1 gstreamer1-plugins-base gstreamer1-plugins-good gstreamer1-plugins-bad-free gstreamer1-plugins-ugly gstreamer1-plugin-libav",
        arch: "sudo pacman -S ffmpeg gst-plugins-base gst-plugins-good gst-plugins-bad gst-plugins-ugly gst-libav",
        suse: "sudo zypper install ffmpeg typelib-1_0-Gst-1_0 gstreamer-plugins-base gstreamer-plugins-good gstreamer-plugins-bad gstreamer-plugins-ugly gstreamer-plugins-libav",
        nixos: "nix-env -iA nixos.ffmpeg nixos.gst_all_1.gstreamer nixos.gst_all_1.gst-plugins-base nixos.gst_all_1.gst-plugins-good nixos.gst_all_1.gst-plugins-bad nixos.gst_all_1.gst-plugins-ugly nixos.gst_all_1.gst-libav"
    };

    let id = GLib.get_os_info('ID') || '';
    let idLike = GLib.get_os_info('ID_LIKE') || '';
    let matchString = `${id} ${idLike}`.toLowerCase();

    if (matchString.includes('ubuntu') || matchString.includes('debian') || matchString.includes('pop') || matchString.includes('mint')) {
        return { name: "Debian / Ubuntu", cmd: instructions.debian };
    } else if (matchString.includes('fedora') || matchString.includes('rhel') || matchString.includes('centos')) {
        return { name: "Fedora", cmd: instructions.fedora };
    } else if (matchString.includes('arch') || matchString.includes('manjaro') || matchString.includes('endeavouros')) {
        return { name: "Arch Linux", cmd: instructions.arch };
    } else if (matchString.includes('suse')) {
        return { name: "openSUSE", cmd: instructions.suse };
    } else if (matchString.includes('nixos')) {
        return { name: "NixOS", cmd: instructions.nixos };
    }

    return { name: "Linux (Generic)", cmd: instructions.debian };
}