// gnome-extension/ui_search.js
import Clutter from 'gi://Clutter';
import GLib from 'gi://GLib';
import St from 'gi://St';
import GObject from 'gi://GObject';

export const GnomeLensSearchBar = GObject.registerClass(
class GnomeLensSearchBar extends St.BoxLayout {
    _init(settings, callbacks) {
        super._init({
            style_class: 'lens-entry-container',
            vertical: false,
        });

        this._settings = settings;
        this.callbacks = callbacks || {};
        this._debounceId = 0;
        this._isSearching = false;

        this._searchIcon = new St.Icon({
            icon_name: 'system-search-symbolic',
            icon_size: 24,
            style_class: 'lens-search-icon',
            y_align: Clutter.ActorAlign.CENTER,
        });
        this._searchIcon.set_style('margin-right: 12px; color: rgba(255, 255, 255, 0.5);');
        this.add_child(this._searchIcon);

        this._entry = new St.Entry({
            style_class: 'lens-entry',
            hint_text: 'Search files, ask the AI...',
            x_expand: true,
            y_align: Clutter.ActorAlign.CENTER,
            can_focus: true,
        });

        this._entry.clutter_text.connectObject('text-changed', this._onTextChanged.bind(this), this);
        this._entry.clutter_text.connectObject('key-press-event', this._onKeyPress.bind(this), this);
        this.add_child(this._entry);

        this._countLabel = new St.Label({
            style_class: 'lens-count-label',
            y_align: Clutter.ActorAlign.CENTER,
            text: '',
            visible: false
        });
        this.add_child(this._countLabel);

        this._closeButton = new St.Button({
            style_class: 'lens-close-button',
            child: new St.Icon({ icon_name: 'window-close-symbolic', icon_size: 24 }),
            y_align: Clutter.ActorAlign.CENTER,
            reactive: true,
            can_focus: true,
        });

        this._closeButton.connectObject('button-press-event', () => {
            if (this.callbacks.onClose) this.callbacks.onClose();
            return Clutter.EVENT_STOP;
        }, this);

        this._closeButton.connectObject('clicked', () => {
            if (this.callbacks.onClose) this.callbacks.onClose();
        }, this);

        this.add_child(this._closeButton);
    }

    _onTextChanged() {
        let text = this._entry.get_text();

        if (this._debounceId > 0) {
            GLib.source_remove(this._debounceId);
            this._debounceId = 0;
        }

        if (text.trim().length < 2) {
            if (this.callbacks.onClear) this.callbacks.onClear();
            return;
        }

        this._debounceId = GLib.timeout_add(GLib.PRIORITY_DEFAULT, 350, () => {
            this._debounceId = 0;
            if (this.callbacks.onSearch) this.callbacks.onSearch(text.trim());
            return GLib.SOURCE_REMOVE;
        });
    }

    _onKeyPress(actor, event) {
        let symbol = event.get_key_symbol();

        if (symbol === Clutter.KEY_Escape) {
            if (this.callbacks.onClose) this.callbacks.onClose();
            return Clutter.EVENT_STOP;
        }

        if (symbol === Clutter.KEY_Down) {
            if (this.callbacks.onNavigateDown) this.callbacks.onNavigateDown();
            return Clutter.EVENT_STOP;
        }

        if (symbol === Clutter.KEY_Up) {
            if (this.callbacks.onNavigateUp) this.callbacks.onNavigateUp();
            return Clutter.EVENT_STOP;
        }

        if (symbol === Clutter.KEY_Return || symbol === Clutter.KEY_KP_Enter) {
            if (this.callbacks.onNavigateEnter) this.callbacks.onNavigateEnter(this._entry.get_text().trim());
            return Clutter.EVENT_STOP;
        }

        return Clutter.EVENT_PROPAGATE;
    }

    setQuery(text, selectAll = true) {
        this._entry.set_text(text);
        if (text.length > 0) {
            GLib.idle_add(GLib.PRIORITY_DEFAULT, () => {
                let len = text.length;
                if (selectAll) {
                    this._entry.clutter_text.set_selection(0, len);
                } else {
                    this._entry.clutter_text.set_selection(len, len);
                }
                return GLib.SOURCE_REMOVE;
            });
        }
    }

    getQuery() {
        return this._entry.get_text() || '';
    }

    grabFocus() {
        this._entry.grab_key_focus();
        this._entry.clutter_text.grab_key_focus();
        
        let text = this._entry.get_text();
        if (text && text.length > 0) {
            GLib.idle_add(GLib.PRIORITY_DEFAULT, () => {
                let len = text.length;
                if (this._settings.get_boolean('select-text-on-focus')) {
                    this._entry.clutter_text.set_selection(0, len);
                } else {
                    this._entry.clutter_text.set_selection(len, len);
                }
                return GLib.SOURCE_REMOVE;
            });
        }
    }

    setCount(count) {
        if (count > 0) {
            this._countLabel.set_text(`${count} results`);
            this._countLabel.show();
        } else {
            this._countLabel.hide();
        }
    }

    startPulse() {
        if (this._isSearching) return;
        this._isSearching = true;
        this._runPulse();
    }

    _runPulse() {
        if (!this._isSearching) return;
        
        this._searchIcon.remove_all_transitions();
        this._searchIcon.ease({
            opacity: 100,
            duration: 600,
            mode: Clutter.AnimationMode.EASE_IN_OUT_QUAD,
            onComplete: () => {
                if (!this._isSearching) return;
                this._searchIcon.ease({
                    opacity: 255,
                    duration: 600,
                    mode: Clutter.AnimationMode.EASE_IN_OUT_QUAD,
                    onComplete: () => {
                        this._runPulse();
                    }
                });
            }
        });
    }

    stopPulse() {
        this._isSearching = false;
        this._searchIcon.remove_all_transitions();
        this._searchIcon.set_opacity(255);
    }

    destroy() {
        if (this._debounceId > 0) {
            GLib.source_remove(this._debounceId);
            this._debounceId = 0;
        }
        this.stopPulse();
        this.disconnectObject(this);
        super.destroy();
    }
});