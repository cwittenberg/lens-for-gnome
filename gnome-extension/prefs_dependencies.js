import GLib from 'gi://GLib';
import GIRepository from 'gi://GIRepository';

export function checkDependencies() {
    let hasFfmpeg = GLib.find_program_in_path('ffmpeg') !== null;
    let repo = GIRepository.Repository.get_default();
    
    let hasGst = repo.enumerate_versions('Gst').length > 0 && repo.enumerate_versions('GstApp').length > 0;
    let hasCogl = repo.enumerate_versions('Cogl').length > 0;
    let hasPdftocairo = GLib.find_program_in_path('pdftocairo') !== null;
    return hasFfmpeg && hasGst && hasCogl && hasPdftocairo;
}

export function checkDaemon() {
    let execPath = GLib.find_program_in_path('lens-for-gnome.daemon') || GLib.find_program_in_path('lens-for-gnome');
    if (execPath) return true;

    let home = GLib.get_home_dir();
    let standardPaths = [
        '/snap/bin/lens-for-gnome.daemon',
        '/snap/bin/lens-for-gnome',
        '/snap/lens-for-gnome/current',
        home + '/.cargo/bin/lens-for-gnome',
        home + '/.local/bin/lens-for-gnome'
    ];

    for (let p of standardPaths) {
        if (GLib.file_test(p, GLib.FileTest.EXISTS)) return true;
    }

    return false;
}

export function getDaemonInstallCommand() {
    return "sudo snap install lens-for-gnome";
}

export function getDistributionInstructions() {
    let instructions = {
        debian: "sudo apt install ffmpeg gir1.2-gstreamer-1.0 gir1.2-cogl-1.0 gstreamer1.0-plugins-base gstreamer1.0-plugins-good gstreamer1.0-plugins-bad gstreamer1.0-plugins-ugly gstreamer1.0-libav poppler-utils",
        fedora: "sudo dnf install ffmpeg gstreamer1 gstreamer1-plugins-base gstreamer1-plugins-good gstreamer1-plugins-bad-free gstreamer1-plugins-ugly gstreamer1-plugin-libav poppler-utils",
        arch: "sudo pacman -S ffmpeg gst-plugins-base gst-plugins-good gst-plugins-bad gst-plugins-ugly gst-libav poppler",
        suse: "sudo zypper install ffmpeg typelib-1_0-Gst-1_0 gstreamer-plugins-base gstreamer-plugins-good gstreamer-plugins-bad gstreamer-plugins-ugly gstreamer-plugins-libav poppler-tools",
        nixos: "nix-env -iA nixos.ffmpeg nixos.gst_all_1.gstreamer nixos.gst_all_1.gst-plugins-base nixos.gst_all_1.gst-plugins-good nixos.gst_all_1.gst-plugins-bad nixos.gst_all_1.gst-plugins-ugly nixos.gst_all_1.gst-libav nixos.poppler_utils"
    };

    let optionalOfficeInstructions = {
        debian: "sudo apt install libreoffice",
        fedora: "sudo dnf install libreoffice",
        arch: "sudo pacman -S libreoffice-fresh",
        suse: "sudo zypper install libreoffice",
        nixos: "nix-env -iA nixos.libreoffice"
    };

    let id = GLib.get_os_info('ID') || '';
    let idLike = GLib.get_os_info('ID_LIKE') || '';
    let matchString = `${id} ${idLike}`.toLowerCase();

    if (matchString.includes('ubuntu') || matchString.includes('debian') || matchString.includes('pop') || matchString.includes('mint')) {
        return { name: "Debian / Ubuntu", cmd: instructions.debian, optionalCmd: optionalOfficeInstructions.debian };
    } else if (matchString.includes('fedora') || matchString.includes('rhel') || matchString.includes('centos')) {
        return { name: "Fedora", cmd: instructions.fedora, optionalCmd: optionalOfficeInstructions.fedora };
    } else if (matchString.includes('arch') || matchString.includes('manjaro') || matchString.includes('endeavouros')) {
        return { name: "Arch Linux", cmd: instructions.arch, optionalCmd: optionalOfficeInstructions.arch };
    } else if (matchString.includes('suse')) {
        return { name: "openSUSE", cmd: instructions.suse, optionalCmd: optionalOfficeInstructions.suse };
    } else if (matchString.includes('nixos')) {
        return { name: "NixOS", cmd: instructions.nixos, optionalCmd: optionalOfficeInstructions.nixos };
    }

    return { name: "Linux (Generic)", cmd: instructions.debian, optionalCmd: optionalOfficeInstructions.debian };
}