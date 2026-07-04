import Adw from 'gi://Adw';
import Gtk from 'gi://Gtk';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import { runtime } from './runtime.js';

function sendDaemonCommand(payloadObj, onMessage) {
    runtime.sendPayload(payloadObj, null, onMessage, 
        () => { if (onMessage) onMessage({ status: 'error', message: 'Offline' }); },
        () => { if (onMessage) onMessage({ status: 'error', message: 'Offline' }); }
    );
}

class MailConfigManager {
    constructor(page) {
        this.page = page;
        this.configPath = runtime.getConfigPath('gmail.json');
        this.pollId = null;
        this._timeoutIds = [];
        this.buildUI();
        this.loadExistingConfig();
    }

    _registerTimeout(id) {
        if (id > 0) this._timeoutIds.push(id);
        return id;
    }

    buildUI() {
        this.gmailGroup = new Adw.PreferencesGroup({ 
             title: 'Gmail Integration',
            description: 'Sync your Gmail inbox locally for instant, private semantic search. Because Google enforces strict security, you must use an App Password, not your standard account password.'
        });

        const helpRow = new Adw.ActionRow({
            title: 'How to get an App Password',
            subtitle: 'Go to Google Account -> Security -> 2-Step Verification -> App Passwords.',
            activatable: true
        });

        const helpIcon = new Gtk.Image({ icon_name: 'external-link-symbolic', valign: Gtk.Align.CENTER });
        helpRow.add_suffix(helpIcon);
        helpRow.connect('activated', () => {
            Gio.AppInfo.launch_default_for_uri('https://myaccount.google.com/apppasswords', null);
        });
        this.gmailGroup.add(helpRow);

        this.emailRow = new Adw.EntryRow({
            title: 'Google Email Address',
            show_apply_button: false
        });
        this.gmailGroup.add(this.emailRow);

        this.passwordRow = new Adw.PasswordEntryRow({
            title: '16-Character App Password',
            show_apply_button: false
        });
        this.gmailGroup.add(this.passwordRow);

        this.historyRow = new Adw.SpinRow({
            title: 'History to Sync (Years)',
            subtitle: 'Limit the initial download to recent emails (Max 5 years).',
            adjustment: new Gtk.Adjustment({
                 lower: 1,
                 upper: 5,
                 step_increment: 1,
                 value: 1
             })
        });
        this.gmailGroup.add(this.historyRow);

        const buttonBox = new Gtk.Box({
            orientation: Gtk.Orientation.HORIZONTAL,
            spacing: 12,
            margin_top: 12,
            margin_bottom: 12,
            margin_start: 12,
            margin_end: 12,
            halign: Gtk.Align.END
        });

        this.statusLabel = new Gtk.Label({
            label: '',
            valign: Gtk.Align.CENTER,
            margin_end: 12
        });
        this.statusLabel.add_css_class('dim-label');

        const clearBtn = new Gtk.Button({
            label: 'Clear',
            valign: Gtk.Align.CENTER
        });
        clearBtn.connect('clicked', () => this.clearConfig());

        const saveBtn = new Gtk.Button({
            label: 'Save & Authenticate',
            valign: Gtk.Align.CENTER
        });
        saveBtn.add_css_class('suggested-action');
        saveBtn.connect('clicked', () => this.saveConfig());

        buttonBox.append(this.statusLabel);
        buttonBox.append(clearBtn);
        buttonBox.append(saveBtn);
        
        this.gmailGroup.add(buttonBox);
        this.page.add(this.gmailGroup);

        this.syncGroup = new Adw.PreferencesGroup({ title: 'Live Sync Status' });
        this.progressRow = new Adw.ActionRow({ title: 'Idle' });
        
        this.progressBox = new Gtk.Box({
            orientation: Gtk.Orientation.HORIZONTAL,
            spacing: 12,
            margin_top: 12,
            margin_bottom: 12,
            margin_start: 12,
            margin_end: 12,
            visible: false
        });
        this.progressBar = new Gtk.ProgressBar({
            show_text: true,
            hexpand: true,
            valign: Gtk.Align.CENTER
        });
        
        this.progressBox.append(this.progressBar);
        this.syncGroup.add(this.progressRow);
        this.syncGroup.add(this.progressBox);
        
        this.page.add(this.syncGroup);

        this.dataGroup = new Adw.PreferencesGroup({ title: 'Data Management' });
        this.resyncRow = new Adw.ActionRow({
            title: 'Force Re-Sync',
            subtitle: 'Forget the last indexed date and download emails from the configured history limit again.'
        });

        this.resyncBtn = new Gtk.Button({
            icon_name: 'view-refresh-symbolic',
            valign: Gtk.Align.CENTER,
            margin_end: 8,
            tooltip_text: 'Reset state and Re-Sync'
        });
        this.resyncBtn.add_css_class('suggested-action');
        this.resyncBtn.connect('clicked', () => {
            this.resyncBtn.set_sensitive(false);
            sendDaemonCommand({ action: 'mail_resync' }, (data) => {
                this._registerTimeout(GLib.timeout_add(GLib.PRIORITY_DEFAULT, 2000, () => {
                    this.resyncBtn.set_sensitive(true);
                    return GLib.SOURCE_REMOVE;
                }));
            });
        });
        this.resyncRow.add_suffix(this.resyncBtn);
        this.dataGroup.add(this.resyncRow);

        this.wipeRow = new Adw.ActionRow({
            title: 'Wipe Local Mail Data',
            subtitle: 'Permanently delete all downloaded .eml files and immediately remove them from the search index.'
        });

        this.wipeBtn = new Gtk.Button({
            icon_name: 'edit-clear-all-symbolic',
            valign: Gtk.Align.CENTER,
            margin_end: 8,
            tooltip_text: 'Wipe Mail Data'
        });
        this.wipeBtn.add_css_class('destructive-action');
        this.wipeBtn.connect('clicked', () => {
            this.wipeBtn.set_sensitive(false);
            sendDaemonCommand({ action: 'mail_wipe' }, (data) => {
                this._registerTimeout(GLib.timeout_add(GLib.PRIORITY_DEFAULT, 2000, () => {
                    this.wipeBtn.set_sensitive(true);
                    return GLib.SOURCE_REMOVE;
                }));
            });
        });

        this.wipeRow.add_suffix(this.wipeBtn);
        this.dataGroup.add(this.wipeRow);
        this.page.add(this.dataGroup);
    }

    startPolling() {
        this.pollId = GLib.timeout_add_seconds(GLib.PRIORITY_DEFAULT, 2, () => {
            sendDaemonCommand({ action: 'get_mail_status' }, (data) => {
                if (data.status === 'mail_status' && data.data) {
                    let state = data.data;
                    
                    this.progressRow.remove_css_class('error-row');
                    this.progressBar.remove_css_class('destructive-action');

                    if (state.is_error) {
                        this.progressBox.set_visible(true);
                        this.progressRow.set_title(`  Sync Fault: ${state.message}`);
                        this.progressBar.add_css_class('destructive-action');
                        this.progressBar.set_fraction(0.0);
                    } else if (state.is_syncing) {
                        this.progressBox.set_visible(true);
                        this.progressRow.set_title(`  ${state.message || 'Syncing entries...'}`);
                        
                        let total = state.total_emails || 1;
                        let current = state.synced_emails || 0;
                        let fraction = current / total;
                        if (fraction > 1.0) fraction = 1.0;
                        
                        this.progressBar.set_fraction(fraction);
                    } else {
                        this.progressBox.set_visible(false);
                        this.progressRow.set_title(`  ${state.message || 'Idle'}`);
                    }
                }
            });
            return GLib.SOURCE_CONTINUE;
        });
    }

    loadExistingConfig() {
        try {
            let file = Gio.File.new_for_path(this.configPath);
            if (file.query_exists(null)) {
                let [success, contents] = file.load_contents(null);
                if (success) {
                    let jsonStr = new TextDecoder().decode(contents);
                    let config = JSON.parse(jsonStr);
                    
                    if (config.email && config.email !== 'your_email@gmail.com') {
                        this.emailRow.set_text(config.email);
                    }
                    if (config.app_password && config.app_password !== 'your_16_char_app_password_here') {
                        this.passwordRow.set_text(config.app_password);
                    }
                    if (config.history_years) {
                        this.historyRow.set_value(config.history_years);
                    }
                }
            }
        } catch (e) {
            console.warn('[Lens for GNOME] Failed to load Gmail config:', e);
        }
    }

    saveConfig() {
        let email = this.emailRow.get_text().trim();
        let password = this.passwordRow.get_text().trim().replace(/[\s\r\n]+/g, '');
        let historyYears = this.historyRow.get_value();

        if (!email || !password) {
            this.statusLabel.set_label('Fields cannot be empty.');
            return;
        }

        if (password.length !== 16) {
            this.statusLabel.set_label('App password must be exactly 16 characters.');
            return;
        }

        let configObj = {
            email: email,
            app_password: password,
            history_years: historyYears
        };

        try {
            let file = Gio.File.new_for_path(this.configPath);
            let parent = file.get_parent();
            if (!parent.query_exists(null)) {
                parent.make_directory_with_parents(null);
            }

            file.replace_contents(
                JSON.stringify(configObj, null, 2),
                null,
                false,
                Gio.FileCreateFlags.REPLACE_DESTINATION,
                null
            );

            let info = new Gio.FileInfo();
            info.set_attribute_uint32('unix::mode', 0o600);
            file.set_attributes_from_info(info, Gio.FileQueryInfoFlags.NONE, null);

            this.statusLabel.set_label('Saved! Background daemon is syncing.');
            
            this._registerTimeout(GLib.timeout_add(GLib.PRIORITY_DEFAULT, 4000, () => {
                this.statusLabel.set_label('');
                return GLib.SOURCE_REMOVE;
            }));

        } catch (e) {
            console.error('[Lens for GNOME] Failed to save Gmail config:', e);
            this.statusLabel.set_label('Error saving configuration.');
        }
    }

    clearConfig() {
        this.emailRow.set_text('');
        this.passwordRow.set_text('');
        this.historyRow.set_value(1);
        
        try {
            let file = Gio.File.new_for_path(this.configPath);
            if (file.query_exists(null)) {
                file.delete(null);
            }
            this.statusLabel.set_label('Credentials cleared.');
        } catch (e) {
            console.warn('[Lens for GNOME] Failed to clear config file:', e);
        }
    }

    destroy() {
        if (this.pollId) {
            GLib.source_remove(this.pollId);
            this.pollId = null;
        }
        if (this._timeoutIds) {
            for (let t of this._timeoutIds) {
                if (t > 0) GLib.source_remove(t);
            }
            this._timeoutIds = [];
        }
    }
}

export function buildMailPage(settings, window) {
    const page = new Adw.PreferencesPage({ 
         title: 'Mail Sync', 
         icon_name: 'mail-unread-symbolic' 
     });

    let manager = new MailConfigManager(page);
    manager.startPolling();

    window.connect('close-request', () => {
        manager.destroy();
    });

    return page;
}