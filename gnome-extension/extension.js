import Meta from 'gi://Meta';
import Shell from 'gi://Shell';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import { Extension } from 'resource:///org/gnome/shell/extensions/extension.js';

import { GnomeLensUI } from './ui.js';
import { GnomeLensIndicator } from './indicator.js';

export default class GnomeLensExtension extends Extension {
    enable() {
        this._settings = this.getSettings('org.gnome.shell.extensions.gnome-lens');
        this._ui = null;
        this._indicator = new GnomeLensIndicator(this, this._settings);
        Main.panel.addToStatusArea('gnome-lens', this._indicator);

        this._settings.connectObject('changed::shortcut', this._bindShortcut.bind(this), this);
        this._bindShortcut();
    }

    disable() {
        Main.wm.removeKeybinding('shortcut');

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
    }

    _bindShortcut() {
        Main.wm.removeKeybinding('shortcut');
        Main.wm.addKeybinding(
            'shortcut',
            this._settings,
            Meta.KeyBindingFlags.NONE,
            Shell.ActionMode.NORMAL | Shell.ActionMode.OVERVIEW,
            this.toggleLens.bind(this)
        );
    }

    toggleLens() {
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