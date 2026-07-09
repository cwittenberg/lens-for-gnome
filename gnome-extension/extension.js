import Meta from 'gi://Meta';
import Shell from 'gi://Shell';
import St from 'gi://St';
import Clutter from 'gi://Clutter';
import GObject from 'gi://GObject';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import * as ModalDialog from 'resource:///org/gnome/shell/ui/modalDialog.js';
import * as Dialog from 'resource:///org/gnome/shell/ui/dialog.js';
import { Extension } from 'resource:///org/gnome/shell/extensions/extension.js';
import { GnomeLensUI } from './ui.js';
import { GnomeLensIndicator } from './indicator.js';
import { checkDaemon, checkDependencies, getDaemonInstallCommand, getDistributionInstructions } from './dependencies.js';
import { runtime } from './runtime.js';

const SetupRequirementsDialog = GObject.registerClass(
class SetupRequirementsDialog extends ModalDialog.ModalDialog {
    _init() {
        super._init({ styleClass: 'lens-setup-dialog' });

        let content = new Dialog.MessageDialogContent({
            title: 'Lens for GNOME   Environment Setup',
            description: 'To enable full functionality, local semantic processing, and rich media previews, please complete the configuration steps detailed below.'
        });
        this.contentLayout.add_child(content);

        let daemonMissing = !checkDaemon();
        let depsMissing = !checkDependencies();

        if (daemonMissing) {
            let daemonTitle = new St.Label({
                text: '1. Install Core Background Ingestion Engine (Snap)',
                style_class: 'lens-setup-section-title'
            });
            this.contentLayout.add_child(daemonTitle);

            let daemonBox = new St.BoxLayout({
                style_class: 'lens-setup-cmd-box',
                vertical: false,
                x_expand: true,
            });

            this._daemonCmdStr = getDaemonInstallCommand();
            let daemonLabel = new St.Label({
                text: this._daemonCmdStr,
                style_class: 'lens-setup-cmd-label',
                x_expand: true
            });
            daemonLabel.clutter_text.line_wrap = true;
            daemonBox.add_child(daemonLabel);

            let daemonCopyBtn = new St.Button({
                label: 'Copy',
                style_class: 'lens-setup-copy-btn',
                y_align: Clutter.ActorAlign.CENTER
            });
            daemonCopyBtn.connectObject('clicked', () => {
                let clipboard = St.Clipboard.get_default();
                clipboard.set_text(St.ClipboardType.CLIPBOARD, this._daemonCmdStr);
                daemonCopyBtn.set_label('Copied!');
            }, this);
            daemonBox.add_child(daemonCopyBtn);

            this.contentLayout.add_child(daemonBox);

            let daemonNoteLabel = new St.Label({
                text: 'Note: The "snap connect" command is needed if you intend to index SMB/NFS drives or other removable media.',
                style_class: 'lens-setup-note-label'
            });
            daemonNoteLabel.clutter_text.line_wrap = true;
            this.contentLayout.add_child(daemonNoteLabel);
        }

        if (depsMissing) {
            let distroInfo = getDistributionInstructions();
            let depsTitle = new St.Label({
                text: `2. Install Multimedia Dependencies (${distroInfo.name})`,
                style_class: 'lens-setup-section-title'
            });
            this.contentLayout.add_child(depsTitle);

            let depsBox = new St.BoxLayout({
                style_class: 'lens-setup-cmd-box',
                vertical: false,
                x_expand: true,
            });

            this._depsCmdStr = distroInfo.cmd;
            let depsLabel = new St.Label({
                text: this._depsCmdStr,
                style_class: 'lens-setup-cmd-label',
                x_expand: true
            });
            depsLabel.clutter_text.line_wrap = true;
            depsBox.add_child(depsLabel);

            let depsCopyBtn = new St.Button({
                label: 'Copy',
                style_class: 'lens-setup-copy-btn',
                y_align: Clutter.ActorAlign.CENTER
            });
            depsCopyBtn.connectObject('clicked', () => {
                let clipboard = St.Clipboard.get_default();
                clipboard.set_text(St.ClipboardType.CLIPBOARD, this._depsCmdStr);
                depsCopyBtn.set_label('Copied!');
            }, this);
            depsBox.add_child(depsCopyBtn);

            this.contentLayout.add_child(depsBox);

            let noteLabel = new St.Label({
                text: 'Note: Restart GNOME Shell after installing multimedia packages to load bindings correctly.',
                style_class: 'lens-setup-note-label'
            });
            noteLabel.clutter_text.line_wrap = true;
            this.contentLayout.add_child(noteLabel);
        }

        this.addButton({
            label: 'Got it',
            action: () => this.close(),
            key: Clutter.KEY_Escape
        });
    }
});

export default class GnomeLensExtension extends Extension {
    enable() {
        this._settings = this.getSettings();
        this._ui = null;
        this._indicator = new GnomeLensIndicator(this, this._settings);
        this._setupDialog = null;
        this._hasShownSetupDialog = false;

        Main.panel.addToStatusArea('lens-for-gnome', this._indicator);

        this._settings.connectObject(
            'changed::shortcut', () => this._bindShortcut(),
            'changed::show-indicator', () => this._updateIndicatorVisibility(),
            this
        );
        this._bindShortcut();
        this._updateIndicatorVisibility();
    }

    disable() {
        Main.wm.removeKeybinding('shortcut');

        if (this._setupDialog) {
            this._setupDialog.destroy();
            this._setupDialog = null;
        }

        if (this._indicator) {
            this._indicator.destroy();
            this._indicator = null;
        }

        if (this._ui) {
            this._ui.destroy();
            this._ui = null;
        }

        if (this._settings) {
            this._settings.disconnectObject(this);
            this._settings = null;
        }

        runtime.destroy();
    }

    _bindShortcut() {
        Main.wm.removeKeybinding('shortcut');
        Main.wm.addKeybinding(
            'shortcut',
            this._settings,
            Meta.KeyBindingFlags.NONE,
            Shell.ActionMode.NORMAL | Shell.ActionMode.OVERVIEW,
            () => this.toggleLens()
        );
    }
    
    _updateIndicatorVisibility() {
        if (this._indicator) {
            this._indicator.visible = this._settings.get_boolean('show-indicator');
        }
    }

    _checkRequirements() {
        let daemonMissing = !checkDaemon();
        let depsMissing = !checkDependencies();

        if (daemonMissing || (depsMissing && !this._hasShownSetupDialog)) {
            if (this._setupDialog) {
                this._setupDialog.destroy();
                this._setupDialog = null;
            }
            
            this._setupDialog = new SetupRequirementsDialog();
            this._setupDialog.open();
            this._hasShownSetupDialog = true;
            
            return false; 
        }
        
        return true;
    }

    toggleLens() {
        if (!this._checkRequirements()) return;

        if (!this._ui) {
            this._ui = new GnomeLensUI(this._settings, this);
        }

        if (this._ui.isOpen) {
            this._ui.close(true);
        } else {
            this._ui.open();
        }
    }

    openLensWithQuery(query) {
        if (!this._checkRequirements()) return;

        if (!this._ui) {
            this._ui = new GnomeLensUI(this._settings, this);
        }

        if (!this._ui.isOpen) {
            this._ui.open();
        }

        this._ui.setQuery(query);
    }

    saveHistory(query) {
        if (!this._settings.get_boolean('enable-history')) {
            return;
        }

        if (!query || query.trim().length === 0) {
            return;
        }

        query = query.trim();
        let history = this._settings.get_strv('search-history') || [];

        let idx = history.indexOf(query);
        if (idx !== -1) {
            history.splice(idx, 1);
        }

        history.unshift(query);

        if (history.length > 10) {
            history.pop();
        }

        this._settings.set_strv('search-history', history);
    }
}