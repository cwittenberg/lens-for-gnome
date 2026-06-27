// gnome-extension/prefs_look_and_feel.js
import Adw from 'gi://Adw';
import Gtk from 'gi://Gtk';
import Gio from 'gi://Gio';
import Gdk from 'gi://Gdk';

export function buildLookAndFeelPage(settings, window) {
    const page = new Adw.PreferencesPage({
        title: 'Look & Feel',
        icon_name: 'preferences-desktop-appearance-symbolic'
    });

    // ==========================================
    // 1. SEARCH BAR GROUP
    // ==========================================
    const searchBarGroup = new Adw.PreferencesGroup({ title: 'Search Bar' });
    
    const selectTextRow = new Adw.SwitchRow({
        title: 'Select Text on Open',
        subtitle: 'Automatically select the previous search text when reopening the window.',
    });
    settings.bind('select-text-on-focus', selectTextRow, 'active', Gio.SettingsBindFlags.DEFAULT);
    searchBarGroup.add(selectTextRow);

    page.add(searchBarGroup);

    // ==========================================
    // 2. RESULTS GROUP
    // ==========================================
    const resultsGroup = new Adw.PreferencesGroup({ title: 'Search Results' });
    
    const prioritizeFoldersRow = new Adw.SwitchRow({
        title: 'Prioritize Folder Results',
        subtitle: 'Ensure that partial folder matches are always pushed to the top of the search list.',
    });
    settings.bind('prioritize-folders', prioritizeFoldersRow, 'active', Gio.SettingsBindFlags.DEFAULT);
    resultsGroup.add(prioritizeFoldersRow);

    const docTextRow = new Adw.SwitchRow({
        title: 'Show Document Text',
        subtitle: 'Display the indexed text snippets for document results by default.',
    });
    settings.bind('show-document-text', docTextRow, 'active', Gio.SettingsBindFlags.DEFAULT);
    resultsGroup.add(docTextRow);
    
    page.add(resultsGroup);

    // ==========================================
    // 3. WINDOW APPEARANCE GROUP
    // ==========================================
    const uiGroup = new Adw.PreferencesGroup({ title: 'Window Appearance' });

    const colorRow = new Adw.ActionRow({
        title: 'Background Color',
        subtitle: 'Select a custom background color for the search window.',
    });

    let initialHex = settings.get_string('ui-color');
    let rgba = new Gdk.RGBA();
    rgba.parse(initialHex);

    let colorButton;
    if (Gtk.ColorDialogButton) {
        let colorDialog = new Gtk.ColorDialog();
        colorButton = new Gtk.ColorDialogButton({
            dialog: colorDialog,
            rgba: rgba,
            valign: Gtk.Align.CENTER
        });
        colorButton.connect('notify::rgba', () => {
            let c = colorButton.get_rgba();
            let r = Math.round(c.red * 255).toString(16).padStart(2, '0');
            let g = Math.round(c.green * 255).toString(16).padStart(2, '0');
            let b = Math.round(c.blue * 255).toString(16).padStart(2, '0');
            settings.set_string('ui-color', `#${r}${g}${b}`);
        });
    } else {
        colorButton = new Gtk.ColorButton({
            rgba: rgba,
            use_alpha: false,
            valign: Gtk.Align.CENTER
        });
        colorButton.connect('color-set', () => {
            let c = colorButton.get_rgba();
            let r = Math.round(c.red * 255).toString(16).padStart(2, '0');
            let g = Math.round(c.green * 255).toString(16).padStart(2, '0');
            let b = Math.round(c.blue * 255).toString(16).padStart(2, '0');
            settings.set_string('ui-color', `#${r}${g}${b}`);
        });
    }

    colorRow.add_suffix(colorButton);
    colorRow.set_activatable_widget(colorButton);
    uiGroup.add(colorRow);

    const transRow = new Adw.SpinRow({
        title: 'Opacity (%)',
        adjustment: new Gtk.Adjustment({ 
            lower: 10, 
            upper: 100, 
            step_increment: 1, 
            value: settings.get_int('ui-transparency') 
        })
    });
    settings.bind('ui-transparency', transRow.adjustment, 'value', Gio.SettingsBindFlags.DEFAULT);
    uiGroup.add(transRow);

    const shadowRow = new Adw.SwitchRow({
        title: 'Window Shadow',
        subtitle: 'Enable drop shadow behind the search window.',
    });
    settings.bind('ui-shadow', shadowRow, 'active', Gio.SettingsBindFlags.DEFAULT);
    uiGroup.add(shadowRow);

    page.add(uiGroup);

    // ==========================================
    // 4. ANIMATIONS GROUP
    // ==========================================
    const animGroup = new Adw.PreferencesGroup({ title: 'Animations' });
    
    const animRow = new Adw.SwitchRow({ title: 'Enable Window Animations' });
    settings.bind('ui-animation', animRow, 'active', Gio.SettingsBindFlags.DEFAULT);
    animGroup.add(animRow);

    const typeModel = Gtk.StringList.new(['Standard', 'Bounce', 'Elastic']);
    const typeRow = new Adw.ComboRow({
        title: 'Animation Type',
        model: typeModel
    });
    
    let currentType = settings.get_string('ui-animation-type');
    if (currentType === 'bounce') typeRow.selected = 1;
    else if (currentType === 'elastic') typeRow.selected = 2;
    else typeRow.selected = 0;

    typeRow.connect('notify::selected', () => {
        let s = typeRow.selected;
        if (s === 1) settings.set_string('ui-animation-type', 'bounce');
        else if (s === 2) settings.set_string('ui-animation-type', 'elastic');
        else settings.set_string('ui-animation-type', 'standard');
    });
    
    animRow.connect('notify::active', () => {
        typeRow.set_sensitive(animRow.active);
    });
    typeRow.set_sensitive(settings.get_boolean('ui-animation'));
    
    animGroup.add(typeRow);

    page.add(animGroup);

    return page;
}