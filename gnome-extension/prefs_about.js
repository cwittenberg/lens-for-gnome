import Adw from 'gi://Adw';
import Gtk from 'gi://Gtk';
import Gio from 'gi://Gio';

export function buildAboutPage() {
    const page = new Adw.PreferencesPage({ 
        title: 'About', 
        icon_name: 'help-about-symbolic' 
    });

    const group = new Adw.PreferencesGroup();

    const titleRow = new Adw.ActionRow({
        title: 'Gnome Lens',
        subtitle: 'Spotlight-alike intelligent search utilizing the local Gnome Lens Daemon.',
    });
    group.add(titleRow);

    const githubRow = new Adw.ActionRow({
        title: 'Source Code',
        subtitle: 'View, report issues, and contribute on GitHub',
        activatable: true
    });
    
    const githubIcon = new Gtk.Image({ 
        icon_name: 'go-next-symbolic', 
        valign: Gtk.Align.CENTER 
    });
    
    githubRow.add_suffix(githubIcon);
    githubRow.connect('activated', () => {
        let uri = 'https://github.com/cwittenberg/gnome-lens';
        Gio.AppInfo.launch_default_for_uri(uri, null);
    });

    group.add(githubRow);
    page.add(group);

    return page;
}