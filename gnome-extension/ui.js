import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import Clutter from 'gi://Clutter';
import St from 'gi://St';
import GObject from 'gi://GObject';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';

import ServiceClient from './service.js';
import { GnomeLensSearchBar, GnomeLensAdvancedFilters } from './ui_search.js';
import { GnomeLensResultsList } from './ui_results.js';
import { GnomeLensSynthesis, GnomeLensStatus } from './ui_status.js';
import { GnomeLensPreview } from './ui_preview.js';

export const GnomeLensUI = GObject.registerClass({
    GTypeName: 'GnomeLensUI',
}, class GnomeLensUI extends St.Widget {
    _init(settings, extension) {
        super._init({
            name: 'GnomeLensBackdrop',
            style_class: 'lens-backdrop',
            reactive: true,
            can_focus: true,
            x: 0,
            y: 0,
            width: 100,
            height: 100,
        });

        this._settings = settings;
        this._extension = extension;
        this._service = new ServiceClient();
        this._activeLaunches = [];
        
        this._historyIndex = -1;
        this._modalGrab = null;
        this._modalPushed = false;
        this._stageCaptureConnected = false;
        
        this.isOpen = false;
        this.isClosing = false;

        this._currentThemeFile = null;
        this._isDragging = false;
        this._dragStartX = 0;
        this._dragStartY = 0;
        this._dragStartWinX = 0;
        this._dragStartWinY = 0;
        this._userMoved = false;

        this._buildUI();
        
        this.connectObject('button-press-event', () => {
            this.close();
            return Clutter.EVENT_STOP;
        }, this);
        
        Main.layoutManager.connectObject('monitors-changed', this._onMonitorsChanged.bind(this), this);
        
        this._settings.connectObject(
            'changed::ui-color', this._applyStyles.bind(this),
            'changed::ui-transparency', this._applyStyles.bind(this),
            'changed::ui-shadow', this._applyStyles.bind(this),
            'changed::ui-theme-path', this._applyTheme.bind(this),
            'changed::show-backdrop', this._applyBackdrop.bind(this),
            'changed::show-document-text', () => {
                if (this._lastResults && this._lastResults.length > 0) {
                    this._resultsList.renderResults(this._lastResults, this._activeFilter);
                }
            },
            this
        );

        this._applyStyles();
        this._applyTheme();
        this._applyBackdrop();
    }

    _applyBackdrop() {
        if (this._settings.get_boolean('show-backdrop')) {
            this.add_style_class_name('lens-backdrop');
        } else {
            this.remove_style_class_name('lens-backdrop');
        }
    }

    _applyTheme() {
        let themePath = this._settings.get_string('ui-theme-path');
        let themeContext = St.ThemeContext.get_for_stage(global.stage);
        
        if (!themeContext) return;
        let theme = themeContext.get_theme();
        if (!theme) return;

        if (this._currentThemeFile) {
            theme.unload_stylesheet(this._currentThemeFile);
            this._currentThemeFile = null;
        }

        if (themePath) {
            let fileToLoad = Gio.File.new_for_path(themePath);
            if (fileToLoad.query_exists(null)) {
                theme.load_stylesheet(fileToLoad);
                this._currentThemeFile = fileToLoad;
            }
        }
    }

    _applyStyles() {
        let color = this._settings.get_string('ui-color');
        let opacity = this._settings.get_int('ui-transparency') / 100.0;
        let shadow = this._settings.get_boolean('ui-shadow');
        
        let bgCss = '';
        
        if (color && /^#[0-9A-Fa-f]{6}$/.test(color)) {
            let r = parseInt(color.slice(1, 3), 16);
            let g = parseInt(color.slice(3, 5), 16);
            let b = parseInt(color.slice(5, 7), 16);
            bgCss = `background-color: rgba(${r}, ${g}, ${b}, ${opacity});`;
        }

        let shadowCss = shadow ? 'box-shadow: 0px 15px 50px rgba(0, 0, 0, 0.5);' : 'box-shadow: none;';
        
        this._dialog.set_style(`${bgCss} ${shadowCss}`);
    }

    _getAnimationParams(baseDuration, isClose = false) {
        if (!this._settings.get_boolean('ui-animation')) {
            return { duration: 0, mode: Clutter.AnimationMode.EASE_OUT_QUAD };
        }

        let type = this._settings.get_string('ui-animation-type');
        let mode = isClose ? Clutter.AnimationMode.EASE_IN_QUAD : Clutter.AnimationMode.EASE_OUT_QUAD;
        
        if (type === 'bounce') {
            mode = isClose ? Clutter.AnimationMode.EASE_IN_BOUNCE : Clutter.AnimationMode.EASE_OUT_BOUNCE;
        } else if (type === 'elastic') {
            mode = isClose ? Clutter.AnimationMode.EASE_IN_ELASTIC : Clutter.AnimationMode.EASE_OUT_ELASTIC;
        }
        
        return { duration: baseDuration, mode: mode };
    }

    _clearSearch() {
        this._service.cancel();
        this._lastResults = [];
        this._activeFilter = 'All';
        this._updateFilterPills([]);
        this._resultsList.clear();
        this._synthesis.setSynthesis(null);
        this._status.stopAnimation();
        this._status.setStatus('');
        this._searchBar.stopPulse();
        this._searchBar.setCount(0);
        this._advancedFilters.clear();
        this._updatePosition(false, true);

        if (this._preview) this._preview.hide();
    }

    _buildUI() {
        this._dialog = new St.BoxLayout({
            vertical: true,
            style_class: 'lens-dialog',
            reactive: true,
        });
        
        this._dialog.set_pivot_point(0.5, 0.5);
        this._dialog.set_scale(0.9, 0.9);
        this._dialog.set_opacity(0);

        this._dialog.connectObject('button-press-event', (actor, event) => {
            if (event.get_button() === 1) {
                let [x, y] = event.get_coords();
                this._isDragging = true;
                this._dragStartX = x;
                this._dragStartY = y;
                this._dragStartWinX = this._dialog.x;
                this._dragStartWinY = this._dialog.y;
            }
            return Clutter.EVENT_STOP;
        }, this);

        this._dialog.connectObject('motion-event', (actor, event) => {
            if (this._isDragging) {
                let [x, y] = event.get_coords();
                let deltaX = x - this._dragStartX;
                let deltaY = y - this._dragStartY;
                this._dialog.set_position(this._dragStartWinX + deltaX, this._dragStartWinY + deltaY);
                this._userMoved = true;
                return Clutter.EVENT_STOP;
            }
            return Clutter.EVENT_PROPAGATE;
        }, this);

        this._dialog.connectObject('button-release-event', (actor, event) => {
            if (event.get_button() === 1 && this._isDragging) {
                this._isDragging = false;
                return Clutter.EVENT_STOP;
            }
            return Clutter.EVENT_PROPAGATE;
        }, this);

        this._searchBar = new GnomeLensSearchBar(this._settings, {
            onClose: () => this.close(true),
            onToggleFilters: () => {
                let isVisible = this._advancedFilters.toggle();
                this._searchBar.toggleFilterActive(isVisible);
            },
            onClear: () => {
                this._clearSearch();
            },
            onSearch: (text) => {
                this._historyIndex = -1;
                this._triggerBackendSearch(text);
            },
            onBack: () => {
                let history = this._settings.get_strv('search-history') || [];
                if (history.length > 0) {
                    if (this._historyIndex < history.length - 1) {
                        this._historyIndex++;
                    }
                    this._loadHistoryAt(this._historyIndex, true);
                }
            },
            onNavigateUp: () => {
                if (this._resultsList.getSelectedIndex() > 0) {
                    this._resultsList.selectPrev();
                } else if (this._resultsList.getSelectedIndex() === 0) {
                    this._resultsList.selectPrev();
                } else if (this._resultsList.getSelectedIndex() === -1) {
                    let history = this._settings.get_strv('search-history') || [];
                    if (this._historyIndex < history.length - 1) {
                        this._historyIndex++;
                        this._resultsList.clear(); 
                        this._loadHistoryAt(this._historyIndex, false);
                    }
                }
            },
            onNavigateDown: () => {
                if (this._resultsList.hasResults()) {
                    if (this._resultsList.getSelectedIndex() === -1) {
                        this._resultsList.selectNext();
                    }
                    this._resultsList.grab_key_focus();
                } else if (this._resultsList.getSelectedIndex() === -1) {
                    if (this._historyIndex > 0) {
                        this._historyIndex--;
                        this._resultsList.clear();
                        this._loadHistoryAt(this._historyIndex, false);
                    } else if (this._historyIndex === 0) {
                        this._historyIndex = -1;
                        this._searchBar.setQuery('', false);
                        this._clearSearch();
                    }
                }
            },
            onNavigateEnter: (query) => {
                if (this._resultsList.getSelectedIndex() !== -1) {
                    this._resultsList.launchSelected();
                } else if (query.length > 0) {
                    this._extension.saveHistory(query);
                }
            },
            isPreviewVideoActive: () => {
                return this._preview && this._preview.isVisible() && this._preview.isVideo();
            },
            onScrub: (offset, isPercentage = false) => {
                if (this._preview && this._preview.isVisible() && typeof this._preview.scrub === 'function') {
                    this._preview.scrub(offset, isPercentage);
                }
            }
        });
        this._dialog.add_child(this._searchBar);

        this._advancedFilters = new GnomeLensAdvancedFilters({
            onFiltersChanged: () => {
                let currentQuery = this._searchBar.getQuery();
                let filterStr = this._advancedFilters.getFilterString();
                if (currentQuery.trim().length > 0 || filterStr.length > 0) {
                    this._triggerBackendSearch(currentQuery);
                } else {
                    this._clearSearch();
                }
            }
        });
        this._dialog.add_child(this._advancedFilters);

        this._activeFilter = 'All';
        this._lastResults = [];

        this._filtersBox = new St.BoxLayout({
            style_class: 'lens-filters-box',
            vertical: false,
            x_align: Clutter.ActorAlign.START,
            visible: false,
        });
        this._dialog.add_child(this._filtersBox);

        this._resultsList = new GnomeLensResultsList(this._settings, {
            onLaunch: (result, action) => this._launchResult(result, action),
            onExploreFolder: (path) => {
                if (path) {
                    let home = GLib.get_home_dir();
                    let displayPath = path;
                    if (displayPath.startsWith(home)) {
                        displayPath = '~' + displayPath.substring(home.length);
                    }
                    if (!displayPath.endsWith('/')) {
                        displayPath += '/';
                    }
                    this._searchBar.setQuery(displayPath, false);
                    this._searchBar.grabFocus();
                    this._triggerBackendSearch(displayPath);
                }
            },
            onSelect: (result) => this._onResultSelected(result),
            onFocusSearch: () => this._searchBar.grabFocus(),
            isPreviewVideoActive: () => {
                return this._preview && this._preview.isVisible() && this._preview.isVideo();
            },
            onScrub: (offset, isPercentage = false) => {
                if (this._preview && this._preview.isVisible() && typeof this._preview.scrub === 'function') {
                    this._preview.scrub(offset, isPercentage);
                }
            }
        });
        this._dialog.add_child(this._resultsList);
        
        this._synthesis = new GnomeLensSynthesis();
        this._resultsList.addSynthesisWidget(this._synthesis);
        
        this._status = new GnomeLensStatus(this._settings);
        this._dialog.add_child(this._status);
        
        this.add_child(this._dialog);
        
        this._preview = new GnomeLensPreview(this._settings);
        this.add_child(this._preview);
        
        this._updatePosition(false, false);
    }

    _onResultSelected(result) {
        if (!this._settings.get_boolean('show-preview')) {
            if (this._preview) this._preview.hide();
            return;
        }

        if (!result || !result.filepath) {
            if (this._preview) this._preview.hide();
            return;
        }

        let ext = result.filepath.split('.').pop().toLowerCase();
        let isVideo = ['mp4', 'mkv', 'avi', 'mov', 'webm', 'flv', 'mpg', 'wmv'].includes(ext);
        let isImage = ['png', 'jpg', 'jpeg', 'gif', 'webp', 'bmp'].includes(ext);

        if (isVideo || isImage) {
            this._preview.showFile(result.filepath, isVideo ? 'video' : 'image');
        } else {
            this._preview.hide();
        }
    }

    _updateFilterPills(results) {
        this._filtersBox.destroy_all_children();
        
        let hasFolders = false, hasApps = false, hasEmails = false, hasFiles = false;
        
        if (results && results.length > 0) {
            results.forEach(r => {
                let ext = '';
                if (r.metadata && r.metadata.filetype) {
                    ext = r.metadata.filetype.toLowerCase();
                } else if (r.filepath) {
                    let parts = r.filepath.split('.');
                    if (parts.length > 1) {
                        ext = parts.pop().toLowerCase();
                    }
                }

                let isFolder = ['directory', 'folder', 'inode/directory'].includes(ext) || r.plugin_id === 'plugin:folder' || r.plugin_id === 'plugin:directory';
                let isApp = r.plugin_id === 'plugin:app_launcher' || r.plugin_id === 'plugin:math';
                let isEmail = r.plugin_id === 'plugin:email' || ext === 'eml';

                if (isFolder) hasFolders = true;
                else if (isApp) hasApps = true;
                else if (isEmail) hasEmails = true;
                else hasFiles = true;
            });
        }

        let options = ['All'];
        if (hasFiles) options.push('Files');
        if (hasFolders) options.push('Folders');
        if (hasApps) options.push('Apps');
        if (hasEmails) options.push('Emails');

        if (options.length > 1) {
            this._filtersBox.show();
            
            let maxAllowedWidth = this._dialog.get_width() - 48; 
            let currentAccumulatedWidth = 0;

            options.forEach(f => {
                let label = new St.Label({ text: f });
                label.clutter_text.ellipsize = 0; 

                let btn = new St.Button({
                    child: label,
                    style_class: 'lens-filter-pill',
                    can_focus: true,
                    reactive: true
                });
                
                if (f === this._activeFilter) {
                    btn.add_style_class_name('active');
                }
                
                btn.connectObject('button-press-event', () => {
                    this._setActiveFilter(f);
                    return Clutter.EVENT_STOP;
                }, this);
                
                let estimatedWidth = f.length * 10 + 32 + 10;

                if (currentAccumulatedWidth + estimatedWidth <= maxAllowedWidth) {
                    this._filtersBox.add_child(btn);
                    currentAccumulatedWidth += estimatedWidth;
                }
            });
        } else {
            this._filtersBox.hide();
        }
    }

    _setActiveFilter(filterName) {
        if (this._activeFilter === filterName) return;
        
        this._activeFilter = filterName;
        this._updateFilterPills(this._lastResults);
        
        if (this._lastResults && this._lastResults.length > 0) {
            this._resultsList.renderResults(this._lastResults, this._activeFilter);
            this._searchBar.setCount(this._resultsList.getCount());
            this._updatePosition(this._resultsList.hasResults(), true);
        }
    }

    _getActiveMonitor() {
        let [x, y] = global.get_pointer();
        
        let monitors = Main.layoutManager.monitors;
        let activeMonitorIndex = monitors.findIndex(m => 
             x >= m.x && x < m.x + m.width &&
             y >= m.y && y < m.y + m.height
        );

        if (activeMonitorIndex >= 0) {
            return monitors[activeMonitorIndex];
        }

        return Main.layoutManager.primaryMonitor;
    }

    _updatePosition(hasResults = false, animate = true) {
        let monitor = this._getActiveMonitor();
        
        this.set_position(monitor.x, monitor.y);
        this.set_size(monitor.width, monitor.height);
        
        let dialogWidth = Math.max(700, Math.min(1000, Math.floor(monitor.width * 0.5)));
        this._dialog.set_width(dialogWidth);
        
        let maxScrollHeight = Math.max(300, Math.min(600, Math.floor(monitor.height * 0.45)));
        this._resultsList.set_style(`max-height: ${maxScrollHeight}px;`);
        
        let targetX = Math.floor((monitor.width - dialogWidth) / 2);
        let targetY = hasResults 
            ? Math.floor(monitor.height * 0.15) 
            : Math.floor(monitor.height * 0.35);

        if (this._userMoved) {
            targetX = this._dialog.x;
            targetY = this._dialog.y;
        }
            
        this._dialog.remove_transition('x');
        this._dialog.remove_transition('y');
        
        let anim = this._getAnimationParams(250, false);

        if (animate && anim.duration > 0) {
            this._dialog.ease({
                x: targetX,
                y: targetY,
                duration: anim.duration,
                mode: anim.mode,
            });
        } else {
            this._dialog.set_position(targetX, targetY);
        }
    }

    _onMonitorsChanged() {
        this._updatePosition(this._resultsList.hasResults(), false);
    }

    _connectStageCapture() {
        if (this._stageCaptureConnected) return;
        global.stage.connectObject('captured-event', this._onCapturedEvent.bind(this), this);
        this._stageCaptureConnected = true;
    }

    _disconnectStageCapture() {
        if (!this._stageCaptureConnected) return;
        global.stage.disconnectObject(this);
        this._stageCaptureConnected = false;
    }

    _onCapturedEvent(actor, event) {
        if (!this.isOpen || this.isClosing) {
            return Clutter.EVENT_PROPAGATE;
        }

        if (event.type() === Clutter.EventType.KEY_PRESS) {
            let symbol = event.get_key_symbol();
            
            if (symbol === Clutter.KEY_Escape) {
                this.close(true);
                return Clutter.EVENT_STOP;
            }
        }
        return Clutter.EVENT_PROPAGATE;
    }

    _pushModal() {
        let grab = Main.pushModal(this);
        this._modalPushed = !!grab;
        this._modalGrab = grab && grab !== true ? grab : null;
    }

    _popModal() {
        if (!this._modalPushed && !this._modalGrab) return;
        
        let grab = this._modalGrab;
        this._modalGrab = null;
        this._modalPushed = false;

        if (grab) {
            Main.popModal(grab);
        } else {
            Main.popModal(this);
        }
    }

    open() {
        if (this.isOpen || this.isClosing) return;
        this.isOpen = true;
        this.isClosing = false;
        
        this.show();
        this.reactive = true;
        this._dialog.reactive = true;

        if (!this.get_parent()) {
            Main.layoutManager.uiGroup.add_child(this);
        }

        this._pushModal();
        this._connectStageCapture();
        
        this._historyIndex = -1;
        this._activeFilter = 'All';
        this._lastResults = [];
        this._userMoved = false;

        this._updateFilterPills([]);
        this._updatePosition(this._resultsList.hasResults(), false);
        
        this._dialog.remove_all_transitions();
        
        let anim = this._getAnimationParams(150, false);

        if (anim.duration > 0) {
            this._dialog.set_scale(0.9, 0.9);
            this._dialog.set_opacity(0);

            this._dialog.ease({
                scale_x: 1.0,
                scale_y: 1.0,
                opacity: 255,
                duration: anim.duration,
                mode: anim.mode,
            });
        } else {
            this._dialog.set_scale(1.0, 1.0);
            this._dialog.set_opacity(255);
        }

        this.grab_key_focus();
        this._searchBar.grabFocus();
    }

    close(instant = false) {
        if (this.isClosing || !this.isOpen) return;
        this.isClosing = true;
        this.reactive = false;
        this._dialog.reactive = false;
        
        if (this._preview) this._preview.hide();
        this._service.cancel();
        this._status.stopAnimation();
        this._searchBar.stopPulse();

        this._disconnectStageCapture();

        global.stage.set_key_focus(null);
        this._popModal();

        this.isOpen = false;

        if (instant) {
            this._finishClose();
            return;
        }

        let anim = this._getAnimationParams(100, true);

        if (anim.duration > 0) {
            this._dialog.remove_all_transitions();
            this._dialog.ease({
                scale_x: 0.9,
                scale_y: 0.9,
                opacity: 0,
                duration: anim.duration,
                mode: anim.mode,
                onComplete: () => {
                    this._finishClose();
                },
            });
        } else {
            this._finishClose();
        }
    }

    _finishClose() {
        this.hide();
        this._dialog.remove_all_transitions();
        this._dialog.set_scale(0.9, 0.9);
        this._dialog.set_opacity(0);

        if (this.get_parent()) {
            Main.layoutManager.uiGroup.remove_child(this);
        }
        this.isClosing = false;
    }

    setQuery(text) {
        this._searchBar.setQuery(text);
    }

    vfunc_key_press_event(keyEvent) {
        if (keyEvent.get_key_symbol() === Clutter.KEY_Escape) {
            this.close(true);
            return Clutter.EVENT_STOP;
        }
        return super.vfunc_key_press_event(keyEvent);
    }

    _loadHistoryAt(index, triggerSearch = true) {
        let history = this._settings.get_strv('search-history') || [];
        if (index >= 0 && index < history.length) {
            let query = history[index];
            this._searchBar.setQuery(query, false);
            if (triggerSearch) {
                this._triggerBackendSearch(query);
            }
        }
    }

    _launchResult(result, action = 'open') {
        this._extension.saveHistory(this._searchBar.getQuery());
        this.close(true);

        let delegationCallback = {
            onMessage: (data) => {
                if (data.status === 'delegate') {
                    let targetPath = data.path;

                    if (data.action === 'open_folder') {
                        let lastSlash = targetPath.lastIndexOf('/');
                        targetPath = lastSlash > 0 ? targetPath.substring(0, lastSlash) : '/';
                    }
                    
                    let uri = targetPath;
                    if (!targetPath.startsWith('http')) {
                        if (!GLib.file_test(targetPath, GLib.FileTest.EXISTS)) {
                            return;
                        }
                        let file = Gio.File.new_for_path(targetPath);
                        uri = file.get_uri();
                    }
                    
                    Gio.AppInfo.launch_default_for_uri_async(
                        uri, 
                        null, 
                        null, 
                        (appInfo, res) => {
                            try {
                                Gio.AppInfo.launch_default_for_uri_finish(res);
                            } catch (e) {
                                console.warn(`[Gnome Lens] Native async launch failed: ${e}`);
                            }
                        }
                    );
                }
            },
            onError: (e) => console.warn(`[Gnome Lens] Launch IPC error: ${e}`),
            onOffline: () => console.warn(`[Gnome Lens] Daemon offline during launch.`)
        };

        let launchService = new ServiceClient();
        this._activeLaunches.push(launchService);

        let cleanup = () => {
            let idx = this._activeLaunches.indexOf(launchService);
            if (idx !== -1) {
                this._activeLaunches.splice(idx, 1);
            }
        };

        let wrappedCallbacks = {
            onMessage: (data) => {
                cleanup();
                if (delegationCallback.onMessage) delegationCallback.onMessage(data);
            },
            onError: (e) => {
                cleanup();
                if (delegationCallback.onError) delegationCallback.onError(e);
            },
            onOffline: () => {
                cleanup();
                if (delegationCallback.onOffline) delegationCallback.onOffline();
            }
        };

        if (result.metadata && result.metadata.gmail_url && action !== 'folder') {
            launchService.sendPayload({ action: 'open_file', path: result.metadata.gmail_url }, wrappedCallbacks);
        } else if (result.plugin_id === 'plugin:app_launcher' && result.metadata && result.metadata.exec) {
            launchService.sendPayload({ action: 'launch_app', exec: result.metadata.exec, filepath: result.filepath || '' }, wrappedCallbacks);
        } else if (result.filepath) {
            if (action === 'folder') {
                launchService.sendPayload({ action: 'open_folder', path: result.filepath }, wrappedCallbacks);
            } else {
                launchService.sendPayload({ action: 'open_file', path: result.filepath }, wrappedCallbacks);
            }
        } else {
            cleanup();
        }
    }

    _triggerBackendSearch(query) {
        let filterStr = this._advancedFilters.getFilterString();
        let fullQuery = query;

        if (filterStr.length > 0) {
            fullQuery = query.trim().length > 0 ? `${query} ${filterStr}` : filterStr;
        }

        if (fullQuery.trim().length === 0) {
            this._clearSearch();
            return;
        }

        this._service.cancel();
        this._searchBar.startPulse();
        this._synthesis.setSynthesis(null);
        if (this._preview) this._preview.hide();
        
        this._activeFilter = 'All';     
        this._updateFilterPills([]);
        
        if (this._resultsList && typeof this._resultsList.clearSelection === 'function') {
            this._resultsList.clearSelection();
        }
        
        let enableAiFiltering = this._settings.get_boolean('enable-ai-filtering');
        let prioritizeFolders = this._settings.get_boolean('prioritize-folders');
        
        this._service.search(fullQuery, enableAiFiltering, prioritizeFolders, {
            onMessage: (data) => {
                if (data.status === 'error') {
                    this._status.setStatus(data.message);
                    this._status.stopAnimation();
                    this._searchBar.stopPulse();
                } else if (data.status === 'filtering' || data.status === 'synthesizing' || data.status === 'processing') {
                    this._status.startAnimation(data.message);
                } else if (data.status === 'done' || data.status === 'final') {
                    this._status.stopAnimation();
                    this._searchBar.stopPulse();

                    if (data.mode !== 'rag_synthesis') {
                        this._synthesis.setSynthesis(null); 
                    }
                }

                if (data.results && Array.isArray(data.results)) {
                    this._lastResults = data.results;
                    
                    this._updateFilterPills(this._lastResults);
                    this._resultsList.renderResults(this._lastResults, this._activeFilter);
                    this._searchBar.setCount(this._resultsList.getCount());
                    
                    if (this._resultsList.hasResults()) {
                        this._updatePosition(true, true);
                    }

                    if (data.mode === 'rag_synthesis' && data.synthesis_result) {
                        this._synthesis.setSynthesis(data.synthesis_result);
                    }
                }
            },
            onOffline: () => {
                this._status.setStatus('Service offline or unreachable.');
                this._searchBar.stopPulse();
            },
            onError: () => {
                this._searchBar.stopPulse();
            }
        });
    }

    destroy() {
        this._disconnectStageCapture();
        this._popModal();
        if (this.isOpen || this.isClosing) {
            this.isOpen = false;
            this.isClosing = false;
            global.stage.set_key_focus(null);
            if (this.get_parent()) {
                Main.layoutManager.uiGroup.remove_child(this);
            }
        }

        if (this._activeLaunches) {
            for (let launch of this._activeLaunches) {
                launch.cancel();
            }
            this._activeLaunches = [];
        }

        if (this._currentThemeFile) {
            let themeContext = St.ThemeContext.get_for_stage(global.stage);
            if (themeContext) {
                let theme = themeContext.get_theme();
                if (theme) {
                    theme.unload_stylesheet(this._currentThemeFile);
                }
            }
            this._currentThemeFile = null;
        }

        if (this._preview) {
            this._preview.destroy();
            this._preview = null;
        }

        this._service.cancel();
        this._settings.disconnectObject(this);
        this.disconnectObject(this);
        Main.layoutManager.disconnectObject(this);

        super.destroy();
    }
});