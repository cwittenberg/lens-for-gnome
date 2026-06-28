// gnome-extension/prefs_about.js
import Adw from 'gi://Adw';
import Gtk from 'gi://Gtk';
import Gio from 'gi://Gio';

function createLinkButton(title, uri, styleClass = null) {
    const button = new Gtk.Button({
        label: title,
        valign: Gtk.Align.CENTER,
        hexpand: true
    });
    
    if (styleClass) {
        button.add_css_class(styleClass);
    }
    
    button.connect('clicked', () => {
        Gio.AppInfo.launch_default_for_uri(uri, null);
    });
    
    return button;
}

export function buildAboutPage(prefs) {
    const page = new Adw.PreferencesPage({ 
        title: 'About', 
        icon_name: 'help-about-symbolic' 
    });

    const group = new Adw.PreferencesGroup();

    const titleRow = new Adw.ActionRow({
        title: 'Gnome Lens',
        subtitle: 'Spotlight-alike intelligent search utilizing the local Gnome Lens Service.',
    });
    group.add(titleRow);

    const linkBox = new Gtk.Box({
        orientation: Gtk.Orientation.HORIZONTAL,
        spacing: 12,
        homogeneous: true,
        halign: Gtk.Align.CENTER,
        margin_top: 16,
        margin_bottom: 16
    });
    
    linkBox.append(createLinkButton('Buy me a coffee \u2615', 'https://ko-fi.com/cwittenberg', 'suggested-action'));
    linkBox.append(createLinkButton('Report a Bug \uD83D\uDC1E', 'https://github.com/cwittenberg/gnome-lens/issues/new?template=bug_report.md'));
    linkBox.append(createLinkButton('Request a Feature \uD83D\uDCA1', 'https://github.com/cwittenberg/gnome-lens/issues/new?template=feature_request.md'));
    
    group.add(linkBox);

    group.add(new Adw.ActionRow({ 
        title: 'Developer', 
        subtitle: 'Christian Wittenberg', 
        title_lines: 0, 
        subtitle_lines: 0 
    }));
    
    let versionStr = 'Local / EGO (Auto-injected)';
    if (prefs && prefs.metadata && prefs.metadata.version !== undefined) {
        versionStr = prefs.metadata.version.toString();
    }
    
    group.add(new Adw.ActionRow({ 
        title: 'Version', 
        subtitle: versionStr, 
        title_lines: 0, 
        subtitle_lines: 0 
    }));

    page.add(group);

    return page;
}