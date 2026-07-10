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
        // Leverages GNOME's native prompt-dialog styling to enforce safe bounds
        super._init({ styleClass: 'prompt-dialog' });

        // Natively handles the text-wrapping bounds via the core shell CSS
        let content = new Dialog.MessageDialogContent({
            title: 'Lens for GNOME Setup',
            description: 'To enable full functionality, please install the missing system dependencies. Copy the commands below and run them in your terminal.'
        });
        this.contentLayout.add_child(content);

        let daemonMissing = !checkDaemon();
        let depsMissing = !checkDependencies();

        if (daemonMissing) {
            let daemonLabel = new St.Label({
                text: '1. Background Ingestion Engine:',
                style: 'font-weight: bold; margin-top: 12px; margin-bottom: 6px;'
            });
            this.contentLayout.add_child(daemonLabel);

            let daemonEntry = new St.Entry({
                text: getDaemonInstallCommand(),
                can_focus: true,
                x_expand: true
            });
            // Auto-select the text so the user can just press Ctrl+C
            daemonEntry.clutter_text.connectObject('button-press-event', () => {
                daemonEntry.clutter_text.set_selection(0, daemonEntry.get_text().length);
                return Clutter.EVENT_PROPAGATE;
            }, this);
            this.contentLayout.add_child(daemonEntry);
        }

        if (depsMissing) {
            let distroInfo = getDistributionInstructions();
            let depsLabel = new St.Label({
                text: `2. Multimedia Dependencies (${distroInfo.name}):`,
                style: 'font-weight: bold; margin-top: 12px; margin-bottom: 6px;'
            });
            this.contentLayout.add_child(depsLabel);

            let depsEntry = new St.Entry({
                text: distroInfo.cmd,
                can_focus: true,
                x_expand: true
            });
            // Auto-select the text so the user can just press Ctrl+C
            depsEntry.clutter_text.connectObject('button-press-event', () => {
                depsEntry.clutter_text.set_selection(0, depsEntry.get_text().length);
                return Clutter.EVENT_PROPAGATE;
            }, this);
            this.contentLayout.add_child(depsEntry);
            
            let noteLabel = new St.Label({
                text: 'Restart GNOME Shell after installing packages.',
                style: 'font-size: 10pt; color: rgba(255,255,255,0.5); margin-top: 8px;'
            });
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
            this._setupDialog.close();
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
                return false; 
            }
            
            this._setupDialog = new SetupRequirementsDialog();
            
            let destroyId = this._setupDialog.connect('destroy', () => {
                this._setupDialog.disconnect(destroyId);
                this._setupDialog = null;
            });
            
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