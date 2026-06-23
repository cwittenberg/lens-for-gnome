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
        });

        this._settings = settings;
        this.callbacks = callbacks || {};
        this._results = [];
        this._resultWidgets = [];
        this._selectedIndex = -1;
        
        this._lastPointerX = -1;
        this._lastPointerY = -1;
        this._ignoreHover = false;

        this._resultsBox = new St.BoxLayout({
            vertical: true,
            x_expand: true,
        });
        
        this.add_child(this._resultsBox);
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
        return this._results.length; 
    }
    
    getSelectedIndex() { 
        return this._selectedIndex; 
    }

    selectNext() {
        if (this._resultWidgets.length > 0 && this._selectedIndex < this._resultWidgets.length - 1) {
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

    launchSelected() {
        if (this._selectedIndex >= 0 && this._selectedIndex < this._results.length) {
            if (this.callbacks.onLaunch) this.callbacks.onLaunch(this._results[this._selectedIndex]);
        }
    }

    _setSelectedIndex(index) {
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

    _fetchThumbnailAsync(filepath, iconActor, fallbackIconName) {
        let file = Gio.File.new_for_path(filepath);
        let uri = file.get_uri();
        let hash = GLib.compute_checksum_for_string(GLib.ChecksumType.MD5, uri, -1);
        
        let paths = [
            GLib.build_filenamev([GLib.get_user_cache_dir(), 'thumbnails', 'normal', hash + '.png']),
            GLib.build_filenamev([GLib.get_user_cache_dir(), 'thumbnails', 'large', hash + '.png']),
            GLib.build_filenamev([GLib.get_user_cache_dir(), 'thumbnails', 'x-large', hash + '.png']),
            GLib.build_filenamev([GLib.get_user_cache_dir(), 'thumbnails', 'xx-large', hash + '.png'])
        ];

        let checkNext = (index) => {
            if (index >= paths.length) return;
            
            let thumbFile = Gio.File.new_for_path(paths[index]);
            thumbFile.query_info_async(Gio.FILE_ATTRIBUTE_STANDARD_TYPE, Gio.FileQueryInfoFlags.NONE, GLib.PRIORITY_DEFAULT, null, (f, res) => {
                try {
                    f.query_info_finish(res);
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

    renderResults(resultsArray) {
        let oldSelectedId = null;
        if (this._selectedIndex >= 0 && this._selectedIndex < this._results.length) {
            oldSelectedId = this._results[this._selectedIndex].id;
        }

        this.clear();
        
        let getExt = (r) => {
            if (r.metadata && r.metadata.filetype) {
                return r.metadata.filetype.toLowerCase();
            } else if (r.filepath) {
                let parts = r.filepath.split('.');
                if (parts.length > 1) {
                    return parts.pop().toLowerCase();
                }
            }
            return '';
        };

        let isFolderResult = (r) => {
            let ext = getExt(r);
            return ['directory', 'folder', 'inode/directory'].includes(ext) || r.plugin_id === 'plugin:folder' || r.plugin_id === 'plugin:directory';
        };

        let getGroup = (r) => {
            if (isFolderResult(r)) return 0;
            if (r.plugin_id === 'plugin:app_launcher' || r.plugin_id === 'plugin:math') return 1;
            if (r.metadata && r.metadata.shallow_index === 'true') return 3;
            return 2;
        };
        
        // Advanced Grouping and Sorting Pipeline
        this._results = [...resultsArray].sort((a, b) => {
            let aMatch = a.ai_matched !== false;
            let bMatch = b.ai_matched !== false;
            if (aMatch !== bMatch) return aMatch ? -1 : 1;

            let groupA = getGroup(a);
            let groupB = getGroup(b);
            
            // 0 (Folders) will always sort before 1, 2, and 3
            if (groupA !== groupB) return groupA - groupB;
            
            return b.score - a.score;
        });

        let maxRender = Math.min(this._results.length, 30);
        let currentGroup = -1;
        let groupNames = ["Folders", "Applications & Tools", "Indexed Documents", "Other Files"];

        for (let i = 0; i < maxRender; i++) {
            let res = this._results[i];
            
            let ext = getExt(res);
            let isFolder = isFolderResult(res);
            let group = getGroup(res);
            
            if (group !== currentGroup) {
                let header = new St.Label({
                    text: groupNames[group],
                    style_class: 'lens-result-group-header'
                });
                this._resultsBox.add_child(header);
                currentGroup = group;
            }

            let itemBox = new St.BoxLayout({
                style_class: 'lens-result-item',
                vertical: false,
                reactive: true,
            });

            if (res.ai_matched === false) {
                itemBox.add_style_class_name('irrelevant');
            }

            itemBox.connectObject('button-press-event', () => {
                if (this.callbacks.onLaunch) this.callbacks.onLaunch(res);
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
                } else if (['mp4', 'mkv', 'webm', 'avi', 'mov', 'flv'].includes(ext)) {
                    isVideoPreview = true;
                    iconName = 'video-x-generic-symbolic';
                } else if (['pdf'].includes(ext)) {
                    isPdfPreview = true;
                    iconName = 'x-office-document-symbolic';
                } else if (['xlsx', 'csv', 'ods'].includes(ext)) {
                    iconName = 'x-office-spreadsheet-symbolic';
                }
            }

            if (res.plugin_id === 'plugin:email') iconName = 'mail-unread-symbolic';
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
                this._fetchThumbnailAsync(res.filepath, iconActor, iconName);
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

            let title = new St.Label({
                text: res.title || 'Unknown Document',
                style_class: 'lens-result-title',
                y_align: Clutter.ActorAlign.CENTER,
            });
            titleBox.add_child(title);

            if (res.filepath && res.plugin_id !== 'plugin:math') {
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
                let reasoningPrefix = res.ai_matched ? '✨ ' : '❌ ';
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

            if (res.filepath && res.plugin_id !== 'plugin:app_launcher' && res.plugin_id !== 'plugin:math') {
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
                let cancellable = new Gio.Cancellable();
                
                itemBox.connect('destroy', () => {
                    if (!cancellable.is_cancelled()) {
                        cancellable.cancel();
                    }
                });

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
                                        } catch(e) { }
                                    });
                                };
                                nextBatch();
                            } catch (e) { }
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
                            } catch (e) { }
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
                openFolderBtn.connectObject('clicked', handleFolderClick, this);
                
                actionBox.add_child(openFolderBtn);
            }

            itemBox.add_child(actionBox);
            this._resultsBox.add_child(itemBox);
            this._resultWidgets.push(itemBox);
        }

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

    destroy() {
        this.clear();
        this.disconnectObject(this);
        super.destroy();
    }
});