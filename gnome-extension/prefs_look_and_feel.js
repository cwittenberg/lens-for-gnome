import Adw from 'gi://Adw';
import Gtk from 'gi://Gtk';
import Gio from 'gi://Gio';
import Gdk from 'gi://Gdk';
import GLib from 'gi://GLib';
import { checkDependencies, getDistributionInstructions } from './dependencies.js';

function showDependencyDialog(parentWindow) {
    let title = "Missing Multimedia Dependencies";
    
    let scrollView = new Gtk.ScrolledWindow({
        min_content_height: 150,
        max_content_height: 400,
        propagate_natural_height: true
    });

    let distroInfo = getDistributionInstructions();
    let pangoBody = `The media preview functionality requires ffmpeg, GStreamer, and Cogl bindings to render video. Please install them for your distribution, then <b>restart GNOME Shell</b>.\n\n<b>${distroInfo.name}</b>\n<tt>${distroInfo.cmd}</tt>`;

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

export function buildLookAndFeelPage(settings, window, extensionPrefs) {
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

    const hasMediaDeps = checkDependencies();
    const previewSwitch = new Gtk.Switch({ valign: Gtk.Align.CENTER });
    const previewRow = new Adw.ActionRow({
        title: 'Show Media Preview Dialog',
        subtitle: hasMediaDeps 
            ? 'Open a draggable video and image preview window when hovering over media files.'
            : 'Dependencies missing. Click to see installation instructions.',
    });
    previewRow.add_suffix(previewSwitch);
    previewRow.set_activatable_widget(previewSwitch);

    if (hasMediaDeps) {
        settings.bind('show-preview', previewSwitch, 'active', Gio.SettingsBindFlags.DEFAULT);
    } else {
        settings.set_boolean('show-preview', false);
        previewSwitch.set_active(false);
        
        previewSwitch.connect('state-set', (sw, state) => {
            if (state) {
                sw.set_state(false); 
                showDependencyDialog(window);
                return true; 
            }
            return false;
        });
    }
    resultsGroup.add(previewRow);
    
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
    if (!initialHex || initialHex.trim() === '') {
        initialHex = '#1e1e1e';
    }
    
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

    const themeRow = new Adw.ActionRow({
        title: 'Custom Theme (.css)',
        subtitle: settings.get_string('ui-theme-path') || 'Default internal theme',
    });

    const selectThemeBtn = new Gtk.Button({
        label: 'Browse...',
        valign: Gtk.Align.CENTER,
    });

    selectThemeBtn.connect('clicked', () => {
        let dialog = new Gtk.FileDialog({ title: 'Select Theme File' });
        
        let filter = new Gtk.FileFilter();
        filter.set_name("Theme Files (*.css)");
        filter.add_pattern("*.css");
        
        let filters = Gio.ListStore.new(Gtk.FileFilter);
        filters.append(filter);
        dialog.set_filters(filters);

        if (extensionPrefs && extensionPrefs.dir) {
            let themesDir = extensionPrefs.dir.get_child('themes');
            if (themesDir.query_exists(null)) {
                dialog.set_initial_folder(themesDir);
            }
        }

        dialog.open(window, null, (dlg, res) => {
            try {
                let file = dlg.open_finish(res);
                if (file) {
                    let path = file.get_path();
                    settings.set_string('ui-theme-path', path);
                    settings.set_string('ui-color', ''); 
                    themeRow.set_subtitle(path);
                }
            } catch (e) {
                console.debug(`[Lens for GNOME] Theme file selection failed or cancelled: ${e.message}`);
            }
        });
    });

    const clearThemeBtn = new Gtk.Button({
        icon_name: 'edit-clear-symbolic',
        valign: Gtk.Align.CENTER,
        margin_start: 8,
        tooltip_text: 'Revert to default theme',
    });
    clearThemeBtn.add_css_class('destructive-action');
    clearThemeBtn.connect('clicked', () => {
        settings.set_string('ui-theme-path', '');
        settings.set_string('ui-color', '#1e1e1e');
        themeRow.set_subtitle('Default internal theme');
    });

    const themeBox = new Gtk.Box({ orientation: Gtk.Orientation.HORIZONTAL });
    themeBox.append(selectThemeBtn);
    themeBox.append(clearThemeBtn);
    
    themeRow.add_suffix(themeBox);
    uiGroup.add(themeRow);

    const updateThemeSensitivity = () => {
        let activeTheme = settings.get_string('ui-theme-path');
        let hasTheme = (activeTheme !== null && activeTheme.trim() !== '');
        
        colorRow.set_sensitive(!hasTheme);
        transRow.set_sensitive(!hasTheme);
    };

    let themePathChangedId = settings.connect('changed::ui-theme-path', updateThemeSensitivity);
    updateThemeSensitivity();

    const shadowRow = new Adw.SwitchRow({
        title: 'Window Shadow',
        subtitle: 'Enable drop shadow behind the search window.',
    });
    settings.bind('ui-shadow', shadowRow, 'active', Gio.SettingsBindFlags.DEFAULT);
    uiGroup.add(shadowRow);

    const backdropRow = new Adw.SwitchRow({
        title: 'Show Screen Overlay',
        subtitle: 'Dim the background screen behind the search window.',
    });
    settings.bind('show-backdrop', backdropRow, 'active', Gio.SettingsBindFlags.DEFAULT);
    uiGroup.add(backdropRow);

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

    window.connect('close-request', () => {
        if (themePathChangedId > 0) {
            settings.disconnect(themePathChangedId);
            themePathChangedId = 0;
        }
    });

    return page;
}