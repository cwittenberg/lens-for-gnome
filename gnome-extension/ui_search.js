// gnome-extension/ui_search.js
import Clutter from 'gi://Clutter';
import GLib from 'gi://GLib';
import St from 'gi://St';
import Gio from 'gi://Gio';
import GObject from 'gi://GObject';

export const GnomeLensAdvancedFilters = GObject.registerClass(
class GnomeLensAdvancedFilters extends St.BoxLayout {
    _init(callbacks) {
        super._init({
            style_class: 'lens-advanced-filters-box',
            vertical: true,
            visible: false,
        });
        this.callbacks = callbacks || {};
        this._isClearing = false;
        this._lastDirText = '';
        this._autocompleteActive = false;

        let row1 = new St.BoxLayout({ 
            vertical: false,
            x_expand: true,
            style_class: 'lens-advanced-filters-row'
        });
        
        this._dirEntry = this._createInput('folder-symbolic', 'Directory (e.g. ~/Docs)');
        this._dirEntry.clutter_text.connectObject('key-press-event', this._onDirKeyPress.bind(this), this);
        this._dirEntry.clutter_text.connectObject('text-changed', this._onDirTextChanged.bind(this), this);
        
        this._extEntry = this._createInput('text-x-generic-symbolic', 'Extension (e.g. pdf)');
        
        row1.add_child(this._dirEntry.get_parent());
        row1.add_child(this._extEntry.get_parent());

        let verticalSpacer = new St.Widget({ height: 20 });

        let row2 = new St.BoxLayout({ 
            vertical: false, 
            x_expand: true,
            y_align: Clutter.ActorAlign.CENTER
        });
        
        let dateLabel = new St.Label({
            text: 'Modified:',
            y_align: Clutter.ActorAlign.CENTER,
            style_class: 'lens-filter-date-label'
        });
        row2.add_child(dateLabel);

        this._datePillsBox = new St.BoxLayout({ vertical: false, style_class: 'lens-date-pills-box' });
        this._dateOptions = [
            { label: 'Any time', days: null },
            { label: 'Today', days: 1 },
            { label: 'Past 7 Days', days: 7 },
            { label: 'Past 30 Days', days: 30 },
            { label: 'This Year', days: 365 }
        ];
        this._activeDateOption = this._dateOptions[0];
        this._datePills = [];

        let currentWidth = 0;
        let maxWidthAllowance = 600;

        this._dateOptions.forEach(opt => {
            let label = new St.Label({ text: opt.label });
            label.clutter_text.ellipsize = 0; 
            
            let pill = new St.Button({
                child: label,
                style_class: 'lens-date-pill',
                can_focus: true,
                reactive: true
            });
            if (opt === this._activeDateOption) pill.add_style_class_name('active');
            
            pill.connectObject('button-press-event', () => {
                this._setDateFilter(opt, pill);
                return Clutter.EVENT_STOP;
            }, this);

            let estimatedWidth = opt.label.length * 10 + 32 + 8;
            if (currentWidth + estimatedWidth <= maxWidthAllowance) {
                this._datePillsBox.add_child(pill);
                this._datePills.push(pill);
                currentWidth += estimatedWidth;
            }
        });
        
        row2.add_child(this._datePillsBox);

        let spacer = new St.Widget({ x_expand: true });
        row2.add_child(spacer);

        let resetLabel = new St.Label({ text: 'Clear Filters' });
        resetLabel.clutter_text.ellipsize = 0;

        this._resetBtn = new St.Button({
            child: resetLabel,
            style_class: 'lens-filter-reset-btn',
            can_focus: true,
            reactive: true
        });
        this._resetBtn.connectObject('button-press-event', () => {
            this.clear();
            if (this.callbacks.onFiltersChanged) this.callbacks.onFiltersChanged();
            return Clutter.EVENT_STOP;
        }, this);
        row2.add_child(this._resetBtn);

        this.add_child(row1);
        this.add_child(verticalSpacer);
        this.add_child(row2);
    }

    _createInput(iconName, hintText) {
        let box = new St.BoxLayout({ 
            style_class: 'lens-filter-input-box', 
            vertical: false, 
            x_expand: true,
            y_align: Clutter.ActorAlign.CENTER
        });
        
        let icon = new St.Icon({ 
            icon_name: iconName, 
            icon_size: 16, 
            style_class: 'lens-filter-icon',
            y_align: Clutter.ActorAlign.CENTER
        });
        box.add_child(icon);
        
        let entry = new St.Entry({ 
            style_class: 'lens-filter-entry', 
            hint_text: hintText, 
            x_expand: true,
            y_align: Clutter.ActorAlign.CENTER
        });
        
        entry.clutter_text.connectObject('text-changed', () => {
            if (!this._isClearing && this.callbacks.onFiltersChanged) {
                this.callbacks.onFiltersChanged();
            }
        }, this);
        
        box.add_child(entry);
        return entry;
    }

    _setDateFilter(option, pillWidget) {
        this._activeDateOption = option;
        this._datePills.forEach(p => p.remove_style_class_name('active'));
        pillWidget.add_style_class_name('active');
        if (!this._isClearing && this.callbacks.onFiltersChanged) {
            this.callbacks.onFiltersChanged();
        }
    }

    _onDirTextChanged() {
        if (this._isClearing) return;
        
        let text = this._dirEntry.get_text();
        this._autocompleteActive = false;

        if (text === this._lastDirText) return;

        if (text.length < this._lastDirText.length) {
            this._lastDirText = text;
            return;
        }
        
        this._lastDirText = text;

        let cursorPos = this._dirEntry.clutter_text.get_cursor_position();
        if (cursorPos !== -1 && cursorPos !== text.length) {
            return;
        }

        let searchPath = text;
        if (searchPath.startsWith('~/')) {
            searchPath = GLib.get_home_dir() + searchPath.slice(1);
        } else if (searchPath === '~') {
            searchPath = GLib.get_home_dir();
        }

        let file = Gio.File.new_for_path(searchPath);
        let parent = file;
        let prefix = '';
        
        if (!text.endsWith('/')) {
            parent = file.get_parent();
            prefix = file.get_basename().toLowerCase();
        }

        if (!parent) return;

        parent.query_info_async(Gio.FILE_ATTRIBUTE_STANDARD_TYPE, Gio.FileQueryInfoFlags.NONE, GLib.PRIORITY_DEFAULT, null, (obj, res) => {
            try {
                let info = obj.query_info_finish(res);
                if (info.get_file_type() !== Gio.FileType.DIRECTORY) return;
                
                parent.enumerate_children_async(
                    'standard::name,standard::type',
                    Gio.FileQueryInfoFlags.NONE,
                    GLib.PRIORITY_DEFAULT,
                    null,
                    (pObj, pRes) => {
                        try {
                            let iter = pObj.enumerate_children_finish(pRes);
                            this._fetchSuggestions(iter, text, prefix);
                        } catch (e) { }
                    }
                );
            } catch (e) { }
        });
    }

    _fetchSuggestions(iter, originalText, prefix) {
        iter.next_files_async(50, GLib.PRIORITY_DEFAULT, null, (obj, res) => {
            try {
                let infos = obj.next_files_finish(res);
                if (!infos || infos.length === 0) {
                    iter.close_async(GLib.PRIORITY_DEFAULT, null, () => {});
                    return;
                }

                let match = null;
                for (let info of infos) {
                    if (info.get_file_type() === Gio.FileType.DIRECTORY) {
                        let name = info.get_name();
                        if (name.startsWith('.') && !prefix.startsWith('.')) continue;
                        if (name.toLowerCase().startsWith(prefix) && name !== prefix) {
                            match = name;
                            break; 
                        }
                    }
                }

                iter.close_async(GLib.PRIORITY_DEFAULT, null, () => {});

                if (match) {
                    // Prevent async overwrites if user continued typing
                    if (this._dirEntry.get_text() !== originalText) return;

                    let newText = originalText;
                    if (!newText.endsWith('/')) {
                        newText = newText.substring(0, newText.lastIndexOf('/') + 1);
                    }
                    newText += match + '/';
                    
                    this._isClearing = true;
                    this._dirEntry.set_text(newText);
                    this._dirEntry.clutter_text.set_selection(originalText.length, newText.length);
                    this._lastDirText = newText;
                    this._autocompleteActive = true;
                    this._isClearing = false;
                }
            } catch (e) { }
        });
    }

    _onDirKeyPress(actor, event) {
        let symbol = event.get_key_symbol();
        let len = this._dirEntry.get_text().length;
        
        if (this._autocompleteActive) {
            if (symbol === Clutter.KEY_Right || symbol === Clutter.KEY_End || symbol === Clutter.KEY_Tab || symbol === Clutter.KEY_Return || symbol === Clutter.KEY_KP_Enter) {
                this._autocompleteActive = false;
                this._isClearing = true;
                this._dirEntry.clutter_text.set_selection(len, len);
                this._isClearing = false;
                
                if (this.callbacks.onFiltersChanged) this.callbacks.onFiltersChanged();
                return Clutter.EVENT_STOP;
            } else {
                this._autocompleteActive = false;
            }
        }
        return Clutter.EVENT_PROPAGATE;
    }

    getFilterString() {
        let parts = [];
        let dir = this._dirEntry.get_text().trim();
        let ext = this._extEntry.get_text().trim();

        if (dir) parts.push(`dir:${dir}`);
        if (ext) parts.push(`ext:${ext}`);
        
        if (this._activeDateOption.days) {
            let d = new Date();
            d.setDate(d.getDate() - this._activeDateOption.days);
            let dateStr = d.toISOString().split('T')[0];
            parts.push(`after:${dateStr}`);
        }

        return parts.join(' ');
    }

    clear() {
        this._isClearing = true;
        
        this._dirEntry.set_text('');
        this._extEntry.set_text('');
        this._lastDirText = '';
        
        this._activeDateOption = this._dateOptions[0];
        this._datePills.forEach(p => p.remove_style_class_name('active'));
        if (this._datePills[0]) {
            this._datePills[0].add_style_class_name('active');
        }
        
        this._isClearing = false;
    }

    toggle() {
        this.visible = !this.visible;
        return this.visible;
    }

    destroy() {
        this.disconnectObject(this);
        super.destroy();
    }
});

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
        this._setQueryIdleId = 0;
        this._focusIdleId = 0;
        this._isSearching = false;
        
        this._isClearing = false;
        this._lastText = '';
        this._autocompleteActive = false;

        this._searchIcon = new St.Icon({
            icon_name: 'system-search-symbolic',
            icon_size: 24,
            style_class: 'lens-search-icon',
            y_align: Clutter.ActorAlign.CENTER,
        });
        this.add_child(this._searchIcon);

        this._entry = new St.Entry({
            style_class: 'lens-entry',
            hint_text: 'Search...',
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

        this._clearButton = new St.Button({
            style_class: 'lens-clear-button',
            child: new St.Icon({ icon_name: 'edit-clear-symbolic', icon_size: 20 }),
            y_align: Clutter.ActorAlign.CENTER,
            reactive: true,
            can_focus: true,
            visible: false,
        });

        this._clearButton.connectObject('button-press-event', () => {
            this.setQuery('');
            if (this.callbacks.onClear) this.callbacks.onClear();
            this.grabFocus();
            return Clutter.EVENT_STOP;
        }, this);
        
        this.add_child(this._clearButton);

        this._filterButton = new St.Button({
            style_class: 'lens-filter-button',
            child: new St.Icon({ icon_name: 'view-more-symbolic', icon_size: 20 }),
            y_align: Clutter.ActorAlign.CENTER,
            reactive: true,
            can_focus: true,
        });

        this._filterButton.connectObject('button-press-event', () => {
            if (this.callbacks.onToggleFilters) this.callbacks.onToggleFilters();
            return Clutter.EVENT_STOP;
        }, this);
        
        this.add_child(this._filterButton);

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
        this.add_child(this._closeButton);
    }

    _onTextChanged() {
        if (this._isClearing) return;

        let text = this._entry.get_text();
        this._autocompleteActive = false; // Reset intercept flag if user typed

        this._clearButton.visible = text.length > 0;

        let cursorPos = this._entry.clutter_text.get_cursor_position();

        // Check if we should fetch autocomplete for paths
        if (text !== this._lastText && text.length > this._lastText.length) {
            if (cursorPos === -1 || cursorPos === text.length) {
                let lastToken = "";
                let tokenStartIdx = -1;
                
                let tildeIdx = text.lastIndexOf(' ~/');
                let slashIdx = text.lastIndexOf(' /');
                let startTilde = text.startsWith('~/') ? 0 : -1;
                let startSlash = text.startsWith('/') ? 0 : -1;
                
                let idx = Math.max(
                    tildeIdx !== -1 ? tildeIdx + 1 : -1,
                    slashIdx !== -1 ? slashIdx + 1 : -1,
                    startTilde,
                    startSlash
                );
                
                if (idx !== -1) {
                    lastToken = text.substring(idx);
                    tokenStartIdx = idx;
                }

                if (lastToken.length > 0) {
                    let searchPath = lastToken;
                    if (searchPath.startsWith('~/')) {
                        searchPath = GLib.get_home_dir() + searchPath.slice(1);
                    } else if (searchPath === '~') {
                        searchPath = GLib.get_home_dir();
                    }

                    let file = Gio.File.new_for_path(searchPath);
                    let parent = file;
                    let prefix = '';
                    
                    if (!lastToken.endsWith('/')) {
                        parent = file.get_parent();
                        prefix = file.get_basename().toLowerCase();
                    }

                    if (parent) {
                        parent.query_info_async(Gio.FILE_ATTRIBUTE_STANDARD_TYPE, Gio.FileQueryInfoFlags.NONE, GLib.PRIORITY_DEFAULT, null, (obj, res) => {
                            try {
                                let info = obj.query_info_finish(res);
                                if (info.get_file_type() === Gio.FileType.DIRECTORY) {
                                    parent.enumerate_children_async(
                                        'standard::name,standard::type',
                                        Gio.FileQueryInfoFlags.NONE,
                                        GLib.PRIORITY_DEFAULT,
                                        null,
                                        (pObj, pRes) => {
                                            try {
                                                let iter = pObj.enumerate_children_finish(pRes);
                                                this._fetchSuggestions(iter, text, prefix, tokenStartIdx);
                                            } catch (e) { }
                                        }
                                    );
                                }
                            } catch (e) { }
                        });
                    }
                }
            }
        }

        this._lastText = text;

        if (this._debounceId > 0) {
            GLib.source_remove(this._debounceId);
            this._debounceId = 0;
        }

        if (text.trim().length < 2) {
            if (this.callbacks.onClear) this.callbacks.onClear();
            return;
        }

        let delay = 350;
        let isDirQuery = (text.startsWith('/') || text.startsWith('~/')) && text.endsWith('/');
        
        // Instant search when they type a trailing slash for a directory
        if (isDirQuery) {
            delay = 0; 
        }

        if (delay === 0) {
            if (this.callbacks.onSearch) this.callbacks.onSearch(text.trim());
        } else {
            this._debounceId = GLib.timeout_add(GLib.PRIORITY_DEFAULT, delay, () => {
                this._debounceId = 0;
                if (this.callbacks.onSearch) this.callbacks.onSearch(text.trim());
                return GLib.SOURCE_REMOVE;
            });
        }
    }

    _fetchSuggestions(iter, originalText, prefix, tokenStartIdx) {
        iter.next_files_async(50, GLib.PRIORITY_DEFAULT, null, (obj, res) => {
            try {
                let infos = obj.next_files_finish(res);
                if (!infos || infos.length === 0) {
                    iter.close_async(GLib.PRIORITY_DEFAULT, null, () => {});
                    return;
                }

                let match = null;
                for (let info of infos) {
                    if (info.get_file_type() === Gio.FileType.DIRECTORY) {
                        let name = info.get_name();
                        if (name.startsWith('.') && !prefix.startsWith('.')) continue;
                        if (name.toLowerCase().startsWith(prefix) && name !== prefix) {
                            match = name;
                            break; 
                        }
                    }
                }

                iter.close_async(GLib.PRIORITY_DEFAULT, null, () => {});

                if (match) {
                    // Critical: if the user typed more while we were fetching asynchronously, abort
                    if (this._entry.get_text() !== originalText) {
                        return;
                    }

                    let newText = originalText;
                    let lastSlash = newText.lastIndexOf('/');
                    if (lastSlash >= tokenStartIdx) {
                        newText = newText.substring(0, lastSlash + 1);
                    } else {
                        newText = newText.substring(0, tokenStartIdx);
                    }
                    newText += match + '/';
                    
                    this._isClearing = true;
                    this._entry.set_text(newText);
                    this._entry.clutter_text.set_selection(originalText.length, newText.length);
                    this._lastText = newText;
                    this._autocompleteActive = true;
                    this._isClearing = false;
                }
            } catch (e) { }
        });
    }

    _onKeyPress(actor, event) {
        let symbol = event.get_key_symbol();
        let text = this._entry.get_text();
        let len = text.length;

        // Catch active autocompletes
        if (this._autocompleteActive) {
            if (symbol === Clutter.KEY_Right || symbol === Clutter.KEY_End || symbol === Clutter.KEY_Tab || symbol === Clutter.KEY_Return || symbol === Clutter.KEY_KP_Enter) {
                this._autocompleteActive = false;
                
                this._isClearing = true;
                this._entry.clutter_text.set_selection(len, len);
                this._isClearing = false;
                
                if (this._debounceId > 0) {
                    GLib.source_remove(this._debounceId);
                    this._debounceId = 0;
                }
                
                if (this.callbacks.onSearch) {
                    this.callbacks.onSearch(text.trim());
                }
                return Clutter.EVENT_STOP;
            } else {
                this._autocompleteActive = false;
            }
        }

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
        
        // Standard explicit search invocation
        if (symbol === Clutter.KEY_Return || symbol === Clutter.KEY_KP_Enter) {
            if (this._debounceId > 0) {
                GLib.source_remove(this._debounceId);
                this._debounceId = 0;
            }
            if (this.callbacks.onSearch) {
                this.callbacks.onSearch(text.trim());
            }
            if (this.callbacks.onNavigateEnter) {
                this.callbacks.onNavigateEnter(text.trim());
            }
            return Clutter.EVENT_STOP;
        }

        return Clutter.EVENT_PROPAGATE;
    }

    setQuery(text, selectAll = true) {
        this._isClearing = true;
        this._entry.set_text(text);
        this._lastText = text;
        this._isClearing = false;
        
        if (text.length > 0) {
            if (this._setQueryIdleId > 0) {
                GLib.source_remove(this._setQueryIdleId);
            }
            this._setQueryIdleId = GLib.idle_add(GLib.PRIORITY_DEFAULT, () => {
                this._setQueryIdleId = 0;
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
            if (this._focusIdleId > 0) {
                GLib.source_remove(this._focusIdleId);
            }
            this._focusIdleId = GLib.idle_add(GLib.PRIORITY_DEFAULT, () => {
                this._focusIdleId = 0;
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

    toggleFilterActive(isActive) {
        if (isActive) {
            this._filterButton.add_style_class_name('active');
        } else {
            this._filterButton.remove_style_class_name('active');
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
        if (this._setQueryIdleId > 0) {
            GLib.source_remove(this._setQueryIdleId);
            this._setQueryIdleId = 0;
        }
        if (this._focusIdleId > 0) {
            GLib.source_remove(this._focusIdleId);
            this._focusIdleId = 0;
        }
        this.stopPulse();
        this.disconnectObject(this);
        super.destroy();
    }
});