// gnome-extension/ui_results.js
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import Clutter from 'gi://Clutter';
import St from 'gi://St';
import GObject from 'gi://GObject';

export const GnomeLensResultsList = GObject.registerClass(
class GnomeLensResultsList extends St.ScrollView {
    _init(settings, callbacks) {
        super._init({
            style_class: 'lens-results-scroll',
            x_expand: true,
            y_expand: true,
            hscrollbar_policy: St.PolicyType.NEVER,
            vscrollbar_policy: St.PolicyType.AUTOMATIC,
            reactive: true,
            can_focus: true
        });
        this._settings = settings;
        this.callbacks = callbacks || {};

        this._results = [];
        this._resultWidgets = [];
        this._selectedIndex = -1;
        
        this._lastPointerX = -1;
        this._lastPointerY = -1;
        this._ignoreHover = false;
        
        // Virtual Scrolling Window Configurations
        this._totalHits = 0;
        this._renderedCount = 0;
        this._chunkSize = 25;
        this._currentGroup = -2;
        this._bestMatchesLength = 0;
        this._scrollConnected = false;

        this._resultsBox = new St.BoxLayout({
            vertical: true,
            x_expand: true,
        });
        
        this.add_child(this._resultsBox);
    }

    vfunc_key_press_event(keyEvent) {
        let symbol = keyEvent.get_key_symbol();
        let state = keyEvent.get_state();
        let isShift = (state & Clutter.ModifierType.SHIFT_MASK) !== 0;
        
        if (symbol === Clutter.KEY_Right) {
            if (this.callbacks.isPreviewVideoActive && this.callbacks.isPreviewVideoActive()) {
                if (this.callbacks.onScrub) this.callbacks.onScrub(isShift ? 0.20 : 5, isShift);
                return Clutter.EVENT_STOP;
            }
        } else if (symbol === Clutter.KEY_Left) {
            if (this.callbacks.isPreviewVideoActive && this.callbacks.isPreviewVideoActive()) {
                if (this.callbacks.onScrub) this.callbacks.onScrub(isShift ? -0.20 : -5, isShift);
                return Clutter.EVENT_STOP;
            }
        }

        if (symbol === Clutter.KEY_Down) {
            this.selectNext();
            return Clutter.EVENT_STOP;
        } else if (symbol === Clutter.KEY_Up) {
            if (this._selectedIndex === 0) {
                if (this.callbacks.onFocusSearch) this.callbacks.onFocusSearch();
            } else {
                this.selectPrev();
            }
            return Clutter.EVENT_STOP;
        } else if (symbol === Clutter.KEY_Return || symbol === Clutter.KEY_KP_Enter) {
            this.launchSelected();
            return Clutter.EVENT_STOP;
        }

        return super.vfunc_key_press_event(keyEvent);
    }

    getResults() {
        return this._results;
    }

    addSynthesisWidget(widget) {
        this._resultsBox.add_child(widget);
    }

    hasResults() { 
        return this._results.length > 0; 
    }
    
    getCount() { 
        if (this._results.length > 0 && this._results[0].metadata && this._results[0].metadata.sys_total_hits) {
            return parseInt(this._results[0].metadata.sys_total_hits, 10);
        }
        return this._results.length; 
    }
    
    getSelectedIndex() { 
        return this._selectedIndex; 
    }

    selectNext() {
        if (this._resultWidgets.length > 0 && this._selectedIndex < this._results.length - 1) {
            this._ignoreHover = true;
            this._setSelectedIndex(this._selectedIndex + 1);
        }
    }

    selectPrev() {
        if (this._selectedIndex > 0) {
            this._ignoreHover = true;
            this._setSelectedIndex(this._selectedIndex - 1);
        } else if (this._selectedIndex === 0) {
            this._ignoreHover = true;
            this._setSelectedIndex(-1);
        }
    }

    _getExt(r) {
        if (r.metadata && r.metadata.filetype) {
            return r.metadata.filetype.toLowerCase();
        } else if (r.filepath) {
            let parts = r.filepath.split('.');
            if (parts.length > 1) {
                return parts.pop().toLowerCase();
            }
        }
        return '';
    }

    _isFolderResult(r) {
        let ext = this._getExt(r);
        return ['directory', 'folder', 'inode/directory'].includes(ext) || r.plugin_id === 'plugin:folder' || r.plugin_id === 'plugin:directory';
    }

    _getGroup(r) {
        let ext = this._getExt(r);
        if (this._isFolderResult(r)) return 0;
        if (r.plugin_id === 'plugin:app_launcher' || r.plugin_id === 'plugin:math') return 1;
        if (r.plugin_id === 'plugin:email' || ext === 'eml') return 2;
        if (r.metadata && r.metadata.shallow_index === 'true') return 4;
        return 3;
    }

    launchSelected() {
        if (this._selectedIndex >= 0 && this._selectedIndex < this._results.length) {
            let res = this._results[this._selectedIndex];
            if (this._isFolderResult(res)) {
                if (this.callbacks.onExploreFolder) {
                    this.callbacks.onExploreFolder(res.filepath);
                } else if (this.callbacks.onLaunch) {
                    this.callbacks.onLaunch(res);
                }
            } else {
                if (this.callbacks.onLaunch) this.callbacks.onLaunch(res);
            }
        }
    }

    clearSelection() {
        this._setSelectedIndex(-1);
    }

    _setSelectedIndex(index) {
        while (index >= this._renderedCount && this._renderedCount < this._results.length) {
            this._renderNextChunk();
        }
        
        if (this._selectedIndex >= 0 && this._selectedIndex < this._resultWidgets.length) {
            this._resultWidgets[this._selectedIndex].remove_style_class_name('selected');
        }
        
        this._selectedIndex = index;
        
        if (this._selectedIndex >= 0 && this._selectedIndex < this._resultWidgets.length) {
            let widget = this._resultWidgets[this._selectedIndex];
            widget.add_style_class_name('selected');

            let adjustment = this.vscroll ? this.vscroll.adjustment : this.vadjustment;
            if (!adjustment) return;
            
            let val = adjustment.value;
            let pageSize = adjustment.page_size;
            let y = widget.allocation.y1;
            let height = widget.allocation.y2 - widget.allocation.y1;

            if (y < val) {
                adjustment.set_value(y);
            } else if (y + height > val + pageSize) {
                adjustment.set_value(y + height - pageSize);
            }
            
            if (this.callbacks.onSelect) {
                this.callbacks.onSelect(this._results[this._selectedIndex]);
            }
        }
    }

    clear() {
        this._results = [];
        this._selectedIndex = -1;
        for (let widget of this._resultWidgets) {
            widget.reactive = false;
            widget.remove_all_transitions();
            widget.destroy();
        }
        this._resultWidgets = [];
        
        let children = this._resultsBox.get_children();
        for (let child of children) {
            if (child.has_style_class_name('lens-result-group-header')) {
                child.destroy();
            }
        }
    }

    _waitForThumbnail(targetPath, iconActor, isImagePreview, originalFile, cancellable) {
        let targetFile = Gio.File.new_for_path(targetPath);
        
        let parent = targetFile.get_parent();
        parent.query_info_async(Gio.FILE_ATTRIBUTE_STANDARD_TYPE, Gio.FileQueryInfoFlags.NONE, GLib.PRIORITY_DEFAULT, cancellable, (f, res) => {
            let exists = false;
            try {
                f.query_info_finish(res);
                exists = true;
            } catch(e) {
                console.debug(`[Lens for GNOME] Target parent directory does not exist: ${e.message}`);
            }
            if (!exists) {
                parent.make_directory_async(GLib.PRIORITY_DEFAULT, cancellable, (pf, pres) => {
                    try { pf.make_directory_finish(pres); } catch(e) { console.debug(`[Lens for GNOME] Failed to make parent directory: ${e.message}`); }
                });
            }
        });

        let monitor;
        try {
            monitor = targetFile.monitor_file(Gio.FileMonitorFlags.NONE, cancellable);
        } catch (e) {
            return;
        }

        let onMonitorEvent = () => {
            if (cancellable && cancellable.is_cancelled()) {
                monitor.cancel();
                return;
            }

            if (targetFile.query_exists(null)) {
                iconActor.set_gicon(new Gio.FileIcon({ file: targetFile }));
                iconActor.set_icon_size(32);
                iconActor.add_style_class_name('lens-result-preview');
                iconActor.remove_style_class_name('lens-result-icon');
                monitor.cancel();
            }
        };

        monitor.connect('changed', onMonitorEvent);

        if (cancellable) {
            cancellable.connect(() => {
                if (monitor && !monitor.is_cancelled()) monitor.cancel();
            });
        }
        
        if (isImagePreview) {
            iconActor.set_gicon(new Gio.FileIcon({ file: originalFile }));
            iconActor.set_icon_size(32);
            iconActor.add_style_class_name('lens-result-preview');
            iconActor.remove_style_class_name('lens-result-icon');
        }
    }

    _fetchThumbnailAsync(filepath, iconActor, fallbackIconName, isImagePreview, cancellable) {
        let file = Gio.File.new_for_path(filepath);
        let uri = file.get_uri();
        let hash = GLib.compute_checksum_for_string(GLib.ChecksumType.MD5, uri, -1);
        
        let cacheDir = GLib.get_user_cache_dir();
        let largeThumbPath = GLib.build_filenamev([cacheDir, 'thumbnails', 'large', hash + '.png']);
        
        let paths = [
            GLib.build_filenamev([cacheDir, 'thumbnails', 'normal', hash + '.png']),
            largeThumbPath,
            GLib.build_filenamev([cacheDir, 'thumbnails', 'x-large', hash + '.png']),
            GLib.build_filenamev([cacheDir, 'thumbnails', 'xx-large', hash + '.png'])
        ];

        let checkNext = (index) => {
            if (cancellable && cancellable.is_cancelled()) return;
            
            if (index >= paths.length) {
                this._waitForThumbnail(largeThumbPath, iconActor, isImagePreview, file, cancellable);
                return;
            }
            
            let thumbFile = Gio.File.new_for_path(paths[index]);
            thumbFile.query_info_async(Gio.FILE_ATTRIBUTE_STANDARD_TYPE, Gio.FileQueryInfoFlags.NONE, GLib.PRIORITY_DEFAULT, cancellable, (f, res) => {
                try {
                    f.query_info_finish(res);
                    if (cancellable && cancellable.is_cancelled()) return;
                    
                    iconActor.set_gicon(new Gio.FileIcon({ file: thumbFile }));
                    iconActor.set_icon_size(32);
                    iconActor.add_style_class_name('lens-result-preview');
                    iconActor.remove_style_class_name('lens-result-icon');
                } catch (e) {
                    checkNext(index + 1);
                }
            });
        };

        checkNext(0);
    }

    _onScroll() {
        let adj = this.vscroll ? this.vscroll.adjustment : this.vadjustment;
        if (!adj) return;
        if (adj.value >= adj.upper - adj.page_size - 150) {
            this._renderNextChunk();
        }
    }

    renderResults(resultsArray, activeFilter = 'All') {
        let oldSelectedId = null;
        if (this._selectedIndex >= 0 && this._selectedIndex < this._results.length) {
            oldSelectedId = this._results[this._selectedIndex].id;
        }

        this.clear();
        
        let sysTotalHits = 0;
        if (resultsArray.length > 0 && resultsArray[0].metadata && resultsArray[0].metadata.sys_total_hits) {
            sysTotalHits = parseInt(resultsArray[0].metadata.sys_total_hits, 10);
        }
        
        let filteredArray = resultsArray.filter(res => {
            if (activeFilter === 'All') return true;
            let group = this._getGroup(res);
            if (activeFilter === 'Folders') return group === 0;
            if (activeFilter === 'Apps') return group === 1;
            if (activeFilter === 'Emails') return group === 2;
            if (activeFilter === 'Files') return group === 3 || group === 4;
            return true;
        });

        let scoreSorted = [...filteredArray].sort((a, b) => {
            let aMatch = a.ai_matched === true;
            let bMatch = b.ai_matched === true;
            if (aMatch !== bMatch) return aMatch ? -1 : 1;

            return (b.score || 0) - (a.score || 0);
        });

        let bestMatches = [];
        let rest = [];

        let isAiFiltered = scoreSorted.length > 0 && scoreSorted.every(r => r.ai_matched === true);

        if (activeFilter === 'All' && scoreSorted.length > 0 && !isAiFiltered) {
            let maxBest = Math.min(5, scoreSorted.length);
            bestMatches = scoreSorted.slice(0, maxBest);
            rest = scoreSorted.slice(maxBest);
            
            rest.sort((a, b) => {
                let aMatch = a.ai_matched === true;
                let bMatch = b.ai_matched === true;
                if (aMatch !== bMatch) return aMatch ? -1 : 1;

                let groupA = this._getGroup(a);
                let groupB = this._getGroup(b);
                if (groupA !== groupB) return groupA - groupB;
                
                return (b.score || 0) - (a.score || 0);
            });
        } else {
            rest = scoreSorted;
            rest.sort((a, b) => {
                let aMatch = a.ai_matched === true;
                let bMatch = b.ai_matched === true;
                if (aMatch !== bMatch) return aMatch ? -1 : 1;

                let groupA = this._getGroup(a);
                let groupB = this._getGroup(b);
                if (groupA !== groupB) return groupA - groupB;
                
                return (b.score || 0) - (a.score || 0);
            });
        }

        this._results = [...bestMatches, ...rest];
        
        if (activeFilter === 'All') {
            this._totalHits = Math.max(sysTotalHits, this._results.length);
        } else {
            this._totalHits = this._results.length;
        }

        this._bestMatchesLength = bestMatches.length;
        this._renderedCount = 0;
        this._currentGroup = -2;
        
        if (!this._scrollConnected) {
            let adj = this.vscroll ? this.vscroll.adjustment : this.vadjustment;
            if (adj) {
                adj.connectObject('notify::value', () => this._onScroll(), this);
                this._scrollConnected = true;
            }
        }

        this._renderNextChunk();

        if (this._results.length > 0) {
            let newIndex = 0;
            if (oldSelectedId) {
                let found = this._results.findIndex(r => r.id === oldSelectedId);
                if (found !== -1 && found < this._resultWidgets.length) {
                    newIndex = found;
                }
            }
            this._setSelectedIndex(newIndex);
        }
    }

    _renderNextChunk() {
        if (this._renderedCount >= this._results.length) return;
        
        let start = this._renderedCount;
        let end = Math.min(start + this._chunkSize, this._results.length);

        let groupNames = ["Folders", "Applications & Tools", "Emails", "Indexed Documents", "Other Files"];

        for (let i = start; i < end; i++) {
            let res = this._results[i];
            let ext = this._getExt(res);
            let isFolder = this._isFolderResult(res);
            let group = this._getGroup(res);
            let isEmail = res.plugin_id === 'plugin:email' || ext === 'eml';
            
            let displayGroup = (i < this._bestMatchesLength) ? -1 : group;
            
            if (displayGroup !== this._currentGroup) {
                let headerText = displayGroup === -1 ? "Top Hits" : (groupNames[group] || "Other");
                let header = new St.Label({
                    text: headerText,
                    style_class: 'lens-result-group-header'
                });
                this._resultsBox.add_child(header);
                this._currentGroup = displayGroup;
            }

            let itemBox = new St.BoxLayout({
                style_class: 'lens-result-item',
                vertical: false,
                reactive: true,
            });

            if (res.ai_matched === false) {
                itemBox.add_style_class_name('irrelevant');
            }
            
            let cancellable = new Gio.Cancellable();
            itemBox.connectObject('destroy', () => {
                if (!cancellable.is_cancelled()) {
                    cancellable.cancel();
                }
            }, this);

            itemBox.connectObject('button-press-event', () => {
                if (isFolder) {
                    if (this.callbacks.onExploreFolder) {
                        this.callbacks.onExploreFolder(res.filepath);
                    } else if (this.callbacks.onLaunch) {
                        this.callbacks.onLaunch(res);
                    }
                } else {
                    if (this.callbacks.onLaunch) this.callbacks.onLaunch(res);
                }
                return Clutter.EVENT_STOP;
            }, this);

            let handlePointerEvent = () => {
                let [x, y] = global.get_pointer();
                
                if (Math.abs(this._lastPointerX - x) > 1 || Math.abs(this._lastPointerY - y) > 1) {
                    this._lastPointerX = x;
                    this._lastPointerY = y;
                    this._ignoreHover = false;
                }

                if (this._ignoreHover) {
                    return Clutter.EVENT_PROPAGATE;
                }

                if (this._selectedIndex !== i) {
                    this._setSelectedIndex(i);
                }
                return Clutter.EVENT_PROPAGATE;
            };

            itemBox.connectObject('enter-event', handlePointerEvent, this);
            itemBox.connectObject('motion-event', handlePointerEvent, this);

            let isImagePreview = false;
            let isVideoPreview = false;
            let isPdfPreview = false;
            let iconName = 'text-x-generic-symbolic';
            let gicon = null;

            if (ext) {
                if (isFolder) {
                    iconName = 'folder-symbolic';
                } else if (['png', 'jpg', 'jpeg', 'bmp', 'webp', 'svg', 'gif'].includes(ext)) {
                    isImagePreview = true;
                    iconName = 'image-x-generic-symbolic';
                } else if (['mp4', 'mkv', 'webm', 'avi', 'mov', 'flv', 'mpg', 'mpeg', 'wmv'].includes(ext)) {
                    isVideoPreview = true;
                    iconName = 'video-x-generic-symbolic';
                } else if (['pdf'].includes(ext)) {
                    isPdfPreview = true;
                    iconName = 'x-office-document-symbolic';
                } else if (['xlsx', 'csv', 'ods'].includes(ext)) {
                    iconName = 'x-office-spreadsheet-symbolic';
                }
            }

            if (isEmail) iconName = 'mail-unread-symbolic';
            if (res.plugin_id === 'plugin:math') iconName = 'accessories-calculator-symbolic';
            
            if (res.plugin_id === 'plugin:app_launcher') {
                if (res.metadata && res.metadata.icon) {
                    if (res.metadata.icon.includes('/')) {
                        let file = Gio.File.new_for_path(res.metadata.icon);
                        gicon = new Gio.FileIcon({ file: file });
                    } else {
                        iconName = res.metadata.icon;
                    }
                } else {
                    iconName = 'application-x-executable-symbolic';
                }
            }

            let iconActor = new St.Icon({
                style_class: 'lens-result-icon',
            });

            if (gicon) {
                iconActor.set_gicon(gicon);
            } else {
                iconActor.set_icon_name(iconName);
            }

            if ((isImagePreview || isVideoPreview || isPdfPreview) && res.filepath) {
                this._fetchThumbnailAsync(res.filepath, iconActor, iconName, isImagePreview, cancellable);
            }

            itemBox.add_child(iconActor);

            let textBox = new St.BoxLayout({
                vertical: true,
                style_class: 'lens-result-text-box',
                y_align: Clutter.ActorAlign.CENTER,
            });

            let titleBox = new St.BoxLayout({
                vertical: false,
                y_align: Clutter.ActorAlign.CENTER,
                x_expand: true,
            });

            let displayTitle = res.title || 'Unknown Document';
            if (isEmail && res.metadata && res.metadata.subject) {
                displayTitle = res.metadata.subject;
            } else if (isEmail) {
                displayTitle = displayTitle.replace('.eml', '');
            }

            let title = new St.Label({
                text: displayTitle,
                style_class: 'lens-result-title',
                y_align: Clutter.ActorAlign.CENTER,
            });
            titleBox.add_child(title);

            if (res.filepath && res.plugin_id !== 'plugin:math' && !isEmail) {
                let parentPathStr = res.filepath;
                let lastSlash = parentPathStr.lastIndexOf('/');
                if (lastSlash > 0) {
                    parentPathStr = parentPathStr.substring(0, lastSlash);
                } else if (lastSlash === 0) {
                    parentPathStr = '/';
                }
                
                let home = GLib.get_home_dir();
                if (parentPathStr.startsWith(home)) {
                    parentPathStr = '~' + parentPathStr.substring(home.length);
                }
                
                let pathLabel = new St.Label({
                    text: 'in ' + parentPathStr,
                    style_class: 'lens-result-path-inline',
                    y_align: Clutter.ActorAlign.CENTER,
                });
                titleBox.add_child(pathLabel);
            }
            textBox.add_child(titleBox);

            let showSnippet = true;
            if (res.plugin_id === 'plugin:vector_db') {
                showSnippet = this._settings.get_boolean('show-document-text');
            }

            if (res.snippet && showSnippet) {
                let cleanSnippet = res.snippet.replace(/<\/?b>/g, '').trim();
                let snippet = new St.Label({
                    text: cleanSnippet.length > 100 ? cleanSnippet.substring(0, 100) + '...' : cleanSnippet,
                    style_class: 'lens-result-snippet',
                });
                textBox.add_child(snippet);
            }

            if (res.ai_reasoning) {
                let reasoningPrefix = res.ai_matched ? '🧠 ' : '❌ ';
                let reasoningLabel = new St.Label({
                    text: reasoningPrefix + res.ai_reasoning,
                    style_class: 'lens-result-ai-reasoning',
                });
                reasoningLabel.clutter_text.line_wrap = true;
                textBox.add_child(reasoningLabel);
            }

            itemBox.add_child(textBox);

            let actionBox = new St.BoxLayout({
                vertical: false,
                style_class: 'lens-result-action-box',
                x_align: Clutter.ActorAlign.END,
                x_expand: true,
                y_align: Clutter.ActorAlign.CENTER,
            });

            if (isEmail) {
                if (res.metadata && res.metadata.from) {
                    let senderPill = new St.BoxLayout({
                        vertical: false,
                        style_class: 'lens-result-folder-pill',
                        y_align: Clutter.ActorAlign.CENTER,
                    });
                    let senderLabel = new St.Label({
                        text: res.metadata.from,
                        style_class: 'lens-result-folder-pill-text',
                        y_align: Clutter.ActorAlign.CENTER,
                    });
                    senderPill.add_child(senderLabel);
                    actionBox.add_child(senderPill);
                }
                
                if (res.metadata && res.metadata.date) {
                    let d = new Date(res.metadata.date);
                    let dateStr = res.metadata.date;
                    if (!isNaN(d.getTime())) {
                        let now = new Date();
                        let isToday = d.getDate() === now.getDate() &&
                                       d.getMonth() === now.getMonth() &&
                                       d.getFullYear() === now.getFullYear();
                        if (isToday) {
                            dateStr = d.toLocaleTimeString([], {hour: '2-digit', minute:'2-digit'});
                        } else {
                            dateStr = d.toLocaleDateString([], {month: 'short', day: 'numeric', year: 'numeric'});
                        }
                    }

                    let datePill = new St.BoxLayout({
                        vertical: false,
                        style_class: 'lens-result-folder-pill',
                        y_align: Clutter.ActorAlign.CENTER,
                    });
                    let dateLabel = new St.Label({
                        text: dateStr,
                        style_class: 'lens-result-folder-pill-text',
                        y_align: Clutter.ActorAlign.CENTER,
                    });
                    datePill.add_child(dateLabel);
                    actionBox.add_child(datePill);
                }
            } else if (res.filepath && res.plugin_id !== 'plugin:app_launcher' && res.plugin_id !== 'plugin:math') {
                let pillBox = new St.BoxLayout({
                    vertical: false,
                    style_class: 'lens-result-folder-pill',
                    y_align: Clutter.ActorAlign.CENTER,
                    visible: false,
                });
                let pillLabel = new St.Label({
                    text: '...',
                    style_class: 'lens-result-folder-pill-text',
                    y_align: Clutter.ActorAlign.CENTER,
                });
                pillBox.add_child(pillLabel);
                actionBox.add_child(pillBox);

                let expandedPath = res.filepath;
                if (expandedPath.startsWith('~/')) {
                    expandedPath = GLib.get_home_dir() + expandedPath.slice(1);
                }
                
                let file = Gio.File.new_for_path(expandedPath);
                
                if (isFolder) {
                    file.enumerate_children_async(
                        'standard::name',
                        Gio.FileQueryInfoFlags.NONE,
                        GLib.PRIORITY_LOW,
                        cancellable,
                        (f, r) => {
                            try {
                                let iter = f.enumerate_children_finish(r);
                                if (cancellable.is_cancelled()) return;
                                
                                let count = 0;
                                let nextBatch = () => {
                                    iter.next_files_async(50, GLib.PRIORITY_LOW, cancellable, (it, queryRes) => {
                                        try {
                                            let files = it.next_files_finish(queryRes);
                                            if (cancellable.is_cancelled()) return;
                                            
                                            if (files && files.length > 0) {
                                                count += files.length;
                                                nextBatch();
                                            } else {
                                                pillLabel.set_text(`${count} items`);
                                                pillBox.show();
                                                it.close_async(GLib.PRIORITY_LOW, null, () => {});
                                            }
                                        } catch(e) { console.debug(`[Lens for GNOME] File batch iteration ignored: ${e.message}`); }
                                    });
                                };
                                nextBatch();
                            } catch (e) { console.debug(`[Lens for GNOME] Folder enumeration failed: ${e.message}`); }
                        }
                    );
                } else {
                    file.query_info_async(
                        Gio.FILE_ATTRIBUTE_STANDARD_SIZE,
                        Gio.FileQueryInfoFlags.NONE,
                        GLib.PRIORITY_LOW,
                        cancellable,
                        (f, r) => {
                            try {
                                let info = f.query_info_finish(r);
                                if (cancellable.is_cancelled()) return;
                                
                                let sizeBytes = info.get_size();
                                let sizes = ['B', 'KB', 'MB', 'GB', 'TB'];
                                let x = 0;
                                let s = sizeBytes;
                                while (s >= 1024 && x < sizes.length - 1) {
                                    s /= 1024;
                                    x++;
                                }
                                let sizeStr = (s < 10 && x > 0 ? s.toFixed(1) : Math.round(s)) + ' ' + sizes[x];
                                pillLabel.set_text(`${sizeStr}`);
                                pillBox.show();
                            } catch (e) { console.debug(`[Lens for GNOME] Could not query filesize: ${e.message}`); }
                        }
                    );
                }
            }

            if (res.filepath) {
                let openFolderBtn = new St.Button({
                    style_class: 'lens-result-action-btn',
                    child: new St.Icon({ icon_name: 'folder-symbolic', icon_size: 20 }),
                    can_focus: true,
                });
                
                let handleFolderClick = () => {
                    if (this.callbacks.onLaunch) this.callbacks.onLaunch(res, 'folder');
                    return Clutter.EVENT_STOP;
                };
                
                openFolderBtn.connectObject('button-press-event', handleFolderClick, this);
                
                actionBox.add_child(openFolderBtn);
            }

            itemBox.add_child(actionBox);
            this._resultsBox.add_child(itemBox);
            this._resultWidgets.push(itemBox);
        }
        
        this._renderedCount = end;
    }
});