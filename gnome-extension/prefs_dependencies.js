// gnome-extension/prefs_dependencies.js
import Adw from 'gi://Adw';
import Gtk from 'gi://Gtk';
import GLib from 'gi://GLib';

export function checkDependencies() {
    let hasFfmpeg = GLib.find_program_in_path('ffmpeg') !== null;
    // GStreamer and Cogl are loaded dynamically and asynchronously in ui_preview_video.js.
    // The preview window will gracefully fall back to ffmpeg frame extraction if 
    // the Gst bindings are not installed, so we only strictly need to verify ffmpeg.
    return hasFfmpeg;
}

function getDistributionInstructions() {
    let instructions = {
        debian: "<b>Debian / Ubuntu</b>\n<tt>sudo apt install ffmpeg gir1.2-gstreamer-1.0 gir1.2-cogl-1.0 gstreamer1.0-plugins-base gstreamer1.0-plugins-good gstreamer1.0-plugins-bad gstreamer1.0-plugins-ugly gstreamer1.0-libav</tt>",
        fedora: "<b>Fedora</b>\n<tt>sudo dnf install ffmpeg gstreamer1 gstreamer1-plugins-base gstreamer1-plugins-good gstreamer1-plugins-bad-free gstreamer1-plugins-ugly gstreamer1-plugin-libav</tt>",
        arch: "<b>Arch Linux</b>\n<tt>sudo pacman -S ffmpeg gst-plugins-base gst-plugins-good gst-plugins-bad gst-plugins-ugly gst-libav</tt>",
        suse: "<b>openSUSE</b>\n<tt>sudo zypper install ffmpeg typelib-1_0-Gst-1_0 gstreamer-plugins-base gstreamer-plugins-good gstreamer-plugins-bad gstreamer-plugins-ugly gstreamer-plugins-libav</tt>",
        nixos: "<b>NixOS</b>\n<tt>ffmpeg gst_all_1.gstreamer gst_all_1.gst-plugins-base gst_all_1.gst-plugins-good gst_all_1.gst-plugins-bad gst_all_1.gst-plugins-ugly gst_all_1.gst-libav</tt>"
    };

    let id = GLib.get_os_info('ID') || '';
    let idLike = GLib.get_os_info('ID_LIKE') || '';
    let matchString = `${id} ${idLike}`.toLowerCase();

    if (matchString.includes('ubuntu') || matchString.includes('debian') || matchString.includes('pop') || matchString.includes('mint')) {
        return instructions.debian;
    } else if (matchString.includes('fedora') || matchString.includes('rhel') || matchString.includes('centos')) {
        return instructions.fedora;
    } else if (matchString.includes('arch') || matchString.includes('manjaro') || matchString.includes('endeavouros')) {
        return instructions.arch;
    } else if (matchString.includes('suse')) {
        return instructions.suse;
    } else if (matchString.includes('nixos')) {
        return instructions.nixos;
    }

    return Object.values(instructions).join("\n\n");
}

export function showDependencyDialog(parentWindow) {
    let title = "Missing Multimedia Dependencies";
    
    let scrollView = new Gtk.ScrolledWindow({
        min_content_height: 150,
        max_content_height: 400,
        propagate_natural_height: true
    });

    let distroInstruction = getDistributionInstructions();
    let pangoBody = "The media preview functionality requires ffmpeg, GStreamer, and Cogl bindings to render video. Please install them for your distribution, then <b>restart GNOME Shell</b>.\n\n" + distroInstruction;

    let label = new Gtk.Label({
        label: pangoBody,
        use_markup: true,
        wrap: true,
        selectable: true,
        xalign: 0,
        margin_top: 12,
        margin_bottom: 12,
        margin_start: 12,
        margin_end: 12
    });

    scrollView.set_child(label);

    if (Adw.AlertDialog) {
        let dialog = new Adw.AlertDialog({
            heading: title,
            extra_child: scrollView
        });
        dialog.add_response('ok', 'Got it');
        dialog.choose(parentWindow, null, () => {});
    } else if (Adw.MessageDialog) {
        let dialog = new Adw.MessageDialog({
            heading: title,
            extra_child: scrollView,
            transient_for: parentWindow
        });
        dialog.add_response('ok', 'Got it');
        dialog.present();
    }
}