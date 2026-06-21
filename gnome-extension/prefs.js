import Adw from 'gi://Adw';
import Gio from 'gi://Gio';
import Gtk from 'gi://Gtk';
import Gdk from 'gi://Gdk';
import { ExtensionPreferences } from 'resource:///org/gnome/Shell/Extensions/js/extensions/prefs.js';

export default class GnomeLensPreferences extends ExtensionPreferences {
    fillPreferencesWindow(window) {
        const settings = this.getSettings('org.gnome.shell.extensions.gnome-lens');
        const page = new Adw.PreferencesPage();
        
        const uiGroup = new Adw.PreferencesGroup({ title: 'User Interface' });
        
        const animationRow = new Adw.SwitchRow({
            title: 'Show LLM Animations',
            subtitle: 'Display bouncing dots while synthesizing data',
        });
        settings.bind('show-llm-animations', animationRow, 'active', Gio.SettingsBindFlags.DEFAULT);
        uiGroup.add(animationRow);
        
        const shortcutRow = new Adw.ActionRow({
            title: 'Activation Shortcut',
            subtitle: 'Click to set a new keyboard shortcut',
        });
        
        const shortcutLabel = new Gtk.ShortcutLabel({
            disabled_text: 'Disabled',
            accelerator: settings.get_strv('shortcut')[0] || '',
            valign: Gtk.Align.CENTER,
        });
        
        const shortcutButton = new Gtk.ToggleButton({
            child: shortcutLabel,
            valign: Gtk.Align.CENTER,
        });
        
        let eventController = new Gtk.EventControllerKey();
        shortcutButton.add_controller(eventController);
        
        eventController.connect('key-pressed', (controller, keyval, keycode, state) => {
            if (!shortcutButton.get_active()) return false;
            
            let mask = state & Gtk.accelerator_get_default_mod_mask();
            mask &= ~Gdk.ModifierType.LOCK_MASK;

            const isEscape = keyval === Gdk.KEY_Escape || keyval === 65307;
            const isBackspace = keyval === Gdk.KEY_BackSpace || keyval === 65288;
            
            if (isEscape) {
                shortcutButton.set_active(false);
                return true;
            }
            
            if (isBackspace) {
                settings.set_strv('shortcut', []);
                shortcutLabel.set_accelerator('');
                shortcutButton.set_active(false);
                return true;
            }
            
            let isModifier = (
                keyval === Gdk.KEY_Control_L || keyval === Gdk.KEY_Control_R ||
                keyval === Gdk.KEY_Shift_L || keyval === Gdk.KEY_Shift_R ||
                keyval === Gdk.KEY_Alt_L || keyval === Gdk.KEY_Alt_R ||
                keyval === Gdk.KEY_Super_L || keyval === Gdk.KEY_Super_R ||
                keyval === Gdk.KEY_Meta_L || keyval === Gdk.KEY_Meta_R ||
                keyval === Gdk.keyval_from_name('Control_L') || keyval === Gdk.keyval_from_name('Control_R') ||
                keyval === Gdk.keyval_from_name('Shift_L') || keyval === Gdk.keyval_from_name('Shift_R') ||
                keyval === Gdk.keyval_from_name('Alt_L') || keyval === Gdk.keyval_from_name('Alt_R') ||
                keyval === Gdk.keyval_from_name('Super_L') || keyval === Gdk.keyval_from_name('Super_R') ||
                keyval === Gdk.keyval_from_name('Meta_L') || keyval === Gdk.keyval_from_name('Meta_R')
            );
            
            if (isModifier) {
                return true; 
            }

            let accelName = Gtk.accelerator_name(keyval, mask);
            
            if (accelName && accelName.length > 0) {
                settings.set_strv('shortcut', [accelName]);
                shortcutLabel.set_accelerator(accelName);
                shortcutButton.set_active(false);
                return true;
            }
            
            return false;
        });
        
        shortcutButton.connect('toggled', () => {
            if (shortcutButton.get_active()) {
                shortcutLabel.set_disabled_text('Press a new shortcut...');
                shortcutLabel.set_accelerator('');
                let win = shortcutButton.get_root();
                if (win) win.focus = shortcutButton;
            } else {
                shortcutLabel.set_disabled_text('Disabled');
                shortcutLabel.set_accelerator(settings.get_strv('shortcut')[0] || '');
            }
        });
        
        shortcutRow.add_suffix(shortcutButton);
        shortcutRow.set_activatable_widget(shortcutButton);
        uiGroup.add(shortcutRow);

        page.add(uiGroup);

        const historyGroup = new Adw.PreferencesGroup({ title: 'Search History' });

        const historySwitch = new Adw.SwitchRow({
            title: 'Enable History',
            subtitle: 'Save recent searches for quick access in the tray context menu',
        });
        settings.bind('enable-history', historySwitch, 'active', Gio.SettingsBindFlags.DEFAULT);
        historyGroup.add(historySwitch);

        const clearBtn = new Gtk.Button({
            label: 'Clear History',
            valign: Gtk.Align.CENTER,
        });
        clearBtn.connect('clicked', () => {
            settings.set_strv('search-history', []);
        });

        const clearRow = new Adw.ActionRow({
            title: 'Clear Saved History',
        });
        clearRow.add_suffix(clearBtn);
        historyGroup.add(clearRow);

        page.add(historyGroup);
        window.add(page);
    }
}