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

// Brutally strips all spaces, non-breaking spaces, newlines, and tabs.
function cleanString(str) {
    if (!str) return '';
    return str.replace(/[\s\u00A0\r\n\t]+/g, '');
}

class MailConfigManager {
    constructor(page, settings) {
        this.page = page;
        this.settings = settings;
        this.pollId = null;
        this._timeoutIds = [];
        this._managedConnections = [];

        this.buildUI();
        this.loadExistingConfig();
    }

    _registerTimeout(id) {
        if (id > 0) this._timeoutIds.push(id);
        return id;
    }

    _trackConnection(widget, signalId) {
        if (signalId > 0) {
            this._managedConnections.push({ widget, signalId });
        }
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
        
        let helpSig = helpRow.connect('activated', () => {
            Gio.AppInfo.launch_default_for_uri('https://myaccount.google.com/apppasswords', null);
        });
        this._trackConnection(helpRow, helpSig);
        
        this.gmailGroup.add(helpRow);

        this.emailRow = new Adw.EntryRow({
            title: 'Google Email Address',
            show_apply_button: false
        });
        
        // INSTANTLY strip spaces on paste/type
        let emailSig = this.emailRow.connect('notify::text', () => {
            let current = this.emailRow.get_text() || '';
            let cleaned = cleanString(current);
            if (current !== cleaned) {
                this.emailRow.set_text(cleaned);
            }
        });
        this._trackConnection(this.emailRow, emailSig);
        
        this.gmailGroup.add(this.emailRow);

        this.passwordRow = new Adw.PasswordEntryRow({
            title: '16-Character App Password',
            show_apply_button: false
        });
        
        // INSTANTLY strip spaces, newlines, and carriage returns on paste
        let passSig = this.passwordRow.connect('notify::text', () => {
            let current = this.passwordRow.get_text() || '';
            let cleaned = cleanString(current);
            if (current !== cleaned) {
                this.passwordRow.set_text(cleaned);
            }
        });
        this._trackConnection(this.passwordRow, passSig);
        
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
        let clearSig = clearBtn.connect('clicked', () => this.clearConfig());
        this._trackConnection(clearBtn, clearSig);

        const saveBtn = new Gtk.Button({
            label: 'Save & Authenticate',
            valign: Gtk.Align.CENTER
        });
        saveBtn.add_css_class('suggested-action');
        let saveSig = saveBtn.connect('clicked', () => this.saveConfig());
        this._trackConnection(saveBtn, saveSig);

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
        
        let resyncSig = this.resyncBtn.connect('clicked', () => {
            this.resyncBtn.set_sensitive(false);
            sendDaemonCommand({ action: 'mail_resync' }, (data) => {
                this._registerTimeout(GLib.timeout_add(GLib.PRIORITY_DEFAULT, 2000, () => {
                    this.resyncBtn.set_sensitive(true);
                    return GLib.SOURCE_REMOVE;
                }));
            });
        });
        this._trackConnection(this.resyncBtn, resyncSig);
        
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
        
        let wipeSig = this.wipeBtn.connect('clicked', () => {
            this.wipeBtn.set_sensitive(false);
            sendDaemonCommand({ action: 'mail_wipe' }, (data) => {
                this._registerTimeout(GLib.timeout_add(GLib.PRIORITY_DEFAULT, 2000, () => {
                    this.wipeBtn.set_sensitive(true);
                    return GLib.SOURCE_REMOVE;
                }));
            });
        });
        this._trackConnection(this.wipeBtn, wipeSig);
        
        this.wipeRow.add_suffix(this.wipeBtn);
        this.dataGroup.add(this.wipeRow);

        this.page.add(this.dataGroup);
    }

    startPolling() {
        const doPoll = () => {
            sendDaemonCommand({ action: 'get_mail_status' }, (data) => {
                let nextInterval = 5; // Default idle interval to prevent UI spam

                if (data.status === 'mail_status' && data.data) {
                    let state = data.data;
                    
                    this.progressRow.remove_css_class('error-row');
                    this.progressBar.remove_css_class('destructive-action');

                    if (state.is_error) {
                        this.progressBox.set_visible(true);
                        this.progressRow.set_title(`  Sync Fault: ${state.message}`);
                        this.progressBar.add_css_class('destructive-action');
                        this.progressBar.set_fraction(0.0);
                        nextInterval = 5; 
                    } else if (state.is_syncing) {
                        this.progressBox.set_visible(true);
                        this.progressRow.set_title(`  ${state.message || 'Syncing entries...'}`);
                        
                        let total = state.total_emails || 1;
                        let current = state.synced_emails || 0;
                        let fraction = current / total;
                        if (fraction > 1.0) fraction = 1.0;
                        
                        this.progressBar.set_fraction(fraction);
                        nextInterval = 1; // Fast polling when active
                    } else {
                        this.progressBox.set_visible(false);
                        this.progressRow.set_title(`  ${state.message || 'Idle'}`);
                    }
                }
                
                // Reschedule next poll dynamically
                this.pollId = GLib.timeout_add_seconds(GLib.PRIORITY_DEFAULT, nextInterval, doPoll);
            });
            return GLib.SOURCE_REMOVE;
        };

        // Kick off initial poll
        this.pollId = GLib.timeout_add_seconds(GLib.PRIORITY_DEFAULT, 1, doPoll);
    }

    loadExistingConfig() {
        let email = this.settings.get_string('mail-account');
        let historyYears = this.settings.get_int('mail-history-years');

        if (email) {
            this.emailRow.set_text(email);
            
            // Fetch password dynamically from the Rust backend via IPC
            sendDaemonCommand({ action: 'get_mail_password', email: email }, (data) => {
                if (data && data.status === 'password_data' && data.password) {
                    this.passwordRow.set_text(data.password);
                }
            });
        }

        if (historyYears) {
            this.historyRow.set_value(historyYears);
        }
    }

    saveConfig() {
        let email = cleanString(this.emailRow.get_text());
        let password = cleanString(this.passwordRow.get_text());
        let historyYears = this.historyRow.get_value();

        console.log(`[UI DEBUG] saveConfig triggered. Email: ${email}, Password Length: ${password.length}`);

        this.emailRow.set_text(email);
        this.passwordRow.set_text(password);

        if (!email || !password) {
            console.log(`[UI DEBUG] Validation failed: Fields cannot be empty.`);
            this.statusLabel.set_label('Fields cannot be empty.');
            return;
        }

        if (password.length !== 16) {
            console.log(`[UI DEBUG] Validation failed: App password must be exactly 16 chars. Length is ${password.length}.`);
            this.statusLabel.set_label(`App password must be exactly 16 chars. (Currently ${password.length})`);
            return;
        }

        this.settings.set_string('mail-account', email);
        this.settings.set_int('mail-history-years', historyYears);
        
        console.log(`[UI DEBUG] Validation passed. Sending IPC command to daemon...`);
        // Rust daemon handles the Secret Service write symmetrically over IPC
        sendDaemonCommand({ 
            action: 'update_mail_config', 
            email: email, 
            password: password,
            history_years: historyYears 
        }, () => {
            console.log(`[UI DEBUG] IPC command acknowledged by daemon.`);
        });

        this.statusLabel.set_label('Saved securely! Daemon is syncing.');
        
        this._registerTimeout(GLib.timeout_add(GLib.PRIORITY_DEFAULT, 4000, () => {
            this.statusLabel.set_label('');
            return GLib.SOURCE_REMOVE;
        }));
    }

    clearConfig() {
        let email = cleanString(this.emailRow.get_text());
        
        this.emailRow.set_text('');
        this.passwordRow.set_text('');
        this.historyRow.set_value(1);
        
        this.settings.set_string('mail-account', '');
        this.settings.set_int('mail-history-years', 1);

        // Sending an empty password commands the Rust daemon to securely delete the keyring entry
        sendDaemonCommand({ 
            action: 'update_mail_config', 
            email: email, 
            password: "",
            history_years: 1 
        }, () => {});

        this.statusLabel.set_label('Credentials cleared.');
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
        
        for (let conn of this._managedConnections) {
            if (conn.widget && conn.signalId > 0) {
                conn.widget.disconnect(conn.signalId);
            }
        }
        this._managedConnections = [];
    }
}

export function buildMailPage(settings, window) {
    const page = new Adw.PreferencesPage({  
        title: 'Mail Sync',  
        icon_name: 'mail-unread-symbolic'  
    });

    let manager = new MailConfigManager(page, settings);
    manager.startPolling();

    window.connect('close-request', () => {
        manager.destroy();
    });

    return page;
}