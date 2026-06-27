// gnome-extension/prefs_index.js
import Adw from 'gi://Adw';
import Gtk from 'gi://Gtk';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';

function sendDaemonCommand(payloadObj, onMessage) {
    let cancellable = new Gio.Cancellable();
    let socketClient = new Gio.SocketClient();
    let socketPath = GLib.get_home_dir() + '/.local/state/gnome-lens/gnome_lens.sock';
    let address = Gio.UnixSocketAddress.new(socketPath);

    const cleanupIPC = (conn, inStream, outStream) => {
        if (inStream) inStream.close_async(GLib.PRIORITY_DEFAULT, null, () => {});
        if (outStream) outStream.close_async(GLib.PRIORITY_DEFAULT, null, () => {});
        if (conn) conn.close_async(GLib.PRIORITY_DEFAULT, null, () => {});
    };

    socketClient.connect_async(address, cancellable, (client, res) => {
        let connection, outputStream;
        try {
            connection = client.connect_finish(res);
            outputStream = connection.get_output_stream();
            let payloadStr = JSON.stringify(payloadObj) + '\n';
            
            outputStream.write_all_async(payloadStr, GLib.PRIORITY_DEFAULT, cancellable, (stream, writeRes) => {
                try {
                    stream.write_all_finish(writeRes);
                    if (onMessage) {
                        let inputStream = new Gio.DataInputStream({ base_stream: connection.get_input_stream() });
                        inputStream.set_newline_type(Gio.DataStreamNewlineType.ANY);
                        
                        let readLoop = function() {
                            inputStream.read_line_async(GLib.PRIORITY_DEFAULT, cancellable, (inStream, inRes) => {
                                try {
                                    let lineData = inStream.read_line_finish_utf8(inRes);
                                    if (lineData && lineData[0] !== null) {
                                        let text = lineData[0].trim();
                                        if (text.length > 0) {
                                            onMessage(JSON.parse(text));
                                        }
                                        readLoop();
                                    } else {
                                        cleanupIPC(connection, inputStream, outputStream);
                                    }
                                } catch (e) {
                                    cleanupIPC(connection, inputStream, outputStream);
                                }
                            });
                        };
                        readLoop();
                    } else {
                        cleanupIPC(connection, null, outputStream);
                    }
                } catch (e) {
                    cleanupIPC(connection, null, outputStream);
                    console.warn("Failed to write command to daemon:", e);
                }
            });
        } catch (e) {
            if (onMessage) {
                onMessage({ status: 'error', message: 'Offline' });
            }
        }
    });
}

function getDaemonExecPath() {
    let execPath = GLib.find_program_in_path('gnome-lens');
    if (execPath) return execPath;

    let home = GLib.get_home_dir();
    
    let standardPaths = [
        home + '/.cargo/bin/gnome-lens',
        home + '/.local/bin/gnome-lens',
        home + '/Development/extensions/gnome-lens/target/release/gnome-lens',
        home + '/Development/extensions/gnome-lens/target/debug/gnome-lens'
    ];

    for (let p of standardPaths) {
        if (GLib.file_test(p, GLib.FileTest.EXISTS)) return p;
    }

    try {
        let [success, stdout] = GLib.spawn_command_line_sync(
            `sh -c "find $HOME/Development $HOME/Projects $HOME/dev $HOME/src $HOME/workspace -maxdepth 5 -type f -name gnome-lens -executable 2>/dev/null | head -n 1"`
        );
        if (success && stdout) {
            let found = new TextDecoder().decode(stdout).trim();
            if (found.length > 0) return found;
        }
    } catch (e) {
        console.warn("[Gnome Lens] Error dynamically searching for dev binary:", e);
    }
    
    return 'gnome-lens';
}

export function buildIndexPage(settings, window) {
    const page = new Adw.PreferencesPage({ 
         title: 'Indexation', 
         icon_name: 'folder-saved-search-symbolic' 
     });

    let isProcessing = false;
    let _timeoutIds = [];

    const safeTimeout = (duration, callback) => {
        let id = GLib.timeout_add(GLib.PRIORITY_DEFAULT, duration, () => {
            callback();
            return GLib.SOURCE_REMOVE;
        });
        _timeoutIds.push(id);
    };

    const serviceGroup = new Adw.PreferencesGroup({ 
         title: 'Background Service Management',
        description: 'Control the lifecycle of the local Gnome Lens ingestion engine process.'
    });

    const statusRow = new Adw.ActionRow({
        title: 'Service Status',
        subtitle: 'Detecting background process state...'
    });

    const spinner = new Gtk.Spinner({
        valign: Gtk.Align.CENTER,
        margin_end: 8,
        visible: false
    });
    statusRow.add_prefix(spinner);

    const startBtn = new Gtk.Button({
        icon_name: 'media-playlist-start-symbolic',
        valign: Gtk.Align.CENTER,
        tooltip_text: 'Start Background Daemon'
    });
    startBtn.add_css_class('suggested-action');

    const stopBtn = new Gtk.Button({
        icon_name: 'media-processor-stop-symbolic',
        valign: Gtk.Align.CENTER,
        tooltip_text: 'Stop Background Daemon'
    });
    stopBtn.add_css_class('destructive-action');

    const restartBtn = new Gtk.Button({
        icon_name: 'view-refresh-symbolic',
        valign: Gtk.Align.CENTER,
        margin_end: 8,
        tooltip_text: 'Restart Background Daemon'
    });

    const updateServiceUI = (silent = false) => {
        if (isProcessing) return; 
        if (!silent) {
            spinner.set_visible(true);
            spinner.start();
        }

        sendDaemonCommand({ action: 'ping' }, (data) => {
            if (isProcessing) return;
            if (!silent) {
                spinner.stop();
                spinner.set_visible(false);
            }
            
            if (data.status === 'error') {
                statusRow.set_title('  Service Status: Stopped');
                statusRow.set_subtitle('The background ingestion engine is inactive.');
                startBtn.set_sensitive(true);
                stopBtn.set_sensitive(false);
                restartBtn.set_sensitive(false);
            } else {
                statusRow.set_title('  Service Status: Running');
                statusRow.set_subtitle('Active and listening for IPC commands.');
                startBtn.set_sensitive(false);
                stopBtn.set_sensitive(true);
                restartBtn.set_sensitive(true);
            }
        });
    };

    startBtn.connect('clicked', () => {
        isProcessing = true;
        startBtn.set_sensitive(false);
        stopBtn.set_sensitive(false);
        restartBtn.set_sensitive(false);
        
        statusRow.set_title('  Service Status: Starting...');
        statusRow.set_subtitle('Booting daemon...');
        spinner.set_visible(true);
        spinner.start();

        try {
            let execPath = getDaemonExecPath();
            let proc = new Gio.Subprocess({
                argv: [execPath],
                flags: Gio.SubprocessFlags.NONE
            });
            proc.init(null);
            
            safeTimeout(1000, () => {
                isProcessing = false;
                updateServiceUI();
                loadStats();
            });
        } catch (e) {
            isProcessing = false;
            spinner.stop();
            spinner.set_visible(false);
            statusRow.set_title('  Launch Failed');
            statusRow.set_subtitle(e.message);
            startBtn.set_sensitive(true);
        }
    });

    stopBtn.connect('clicked', () => {
        isProcessing = true;
        startBtn.set_sensitive(false);
        stopBtn.set_sensitive(false);
        restartBtn.set_sensitive(false);
        
        statusRow.set_title('  Service Status: Stopping...');
        statusRow.set_subtitle('Shutting down gracefully...');
        spinner.set_visible(true);
        spinner.start();

        sendDaemonCommand({ action: 'shutdown' }, () => {});
        
        safeTimeout(1000, () => {
            isProcessing = false;
            updateServiceUI();
        });
    });

    restartBtn.connect('clicked', () => {
        isProcessing = true;
        startBtn.set_sensitive(false);
        stopBtn.set_sensitive(false);
        restartBtn.set_sensitive(false);
        
        statusRow.set_title('  Service Status: Restarting...');
        statusRow.set_subtitle('Cycling daemon processes...');
        spinner.set_visible(true);
        spinner.start();

        sendDaemonCommand({ action: 'shutdown' }, () => {});
        
        safeTimeout(1500, () => {
            try {
                let execPath = getDaemonExecPath();
                let proc = new Gio.Subprocess({
                    argv: [execPath],
                    flags: Gio.SubprocessFlags.NONE
                });
                proc.init(null);
            } catch (e) {
                console.warn("Restart failed to launch binary:", e);
                statusRow.set_title('  Launch Failed');
                statusRow.set_subtitle(e.message);
                spinner.stop();
                spinner.set_visible(false);
                isProcessing = false;
                startBtn.set_sensitive(true);
                return;
            }

            safeTimeout(1000, () => {
                isProcessing = false;
                updateServiceUI();
                loadStats();
            });
        });
    });

    statusRow.add_suffix(restartBtn);
    statusRow.add_suffix(startBtn);
    statusRow.add_suffix(stopBtn);
    serviceGroup.add(statusRow);
    page.add(serviceGroup);

    // ==========================================
    // LIVE INGESTION PROGRESS TRACKER
    // ==========================================
    const progressGroup = new Adw.PreferencesGroup({ title: 'Live Ingestion Progress' });
    const progressRow = new Adw.ActionRow({ title: 'Idle', subtitle: 'System is resting or listening for changes.' });
    
    const progressBox = new Gtk.Box({
        orientation: Gtk.Orientation.HORIZONTAL,
        spacing: 12,
        margin_top: 12,
        margin_bottom: 12,
        margin_start: 12,
        margin_end: 12,
        visible: false
    });
    const progressBar = new Gtk.ProgressBar({
        hexpand: true,
        valign: Gtk.Align.CENTER
    });
    progressBar.set_inverted(false);
    progressBox.append(progressBar);

    progressGroup.add(progressRow);
    progressGroup.add(progressBox);
    page.add(progressGroup);

    let healthCheckId = GLib.timeout_add_seconds(GLib.PRIORITY_DEFAULT, 1, () => {
        updateServiceUI(true);
        
        sendDaemonCommand({ action: 'get_indexer_status' }, (data) => {
            if (data.status === 'indexer_status' && data.data) {
                let state = data.data;
                if (state.is_running) {
                    progressBox.set_visible(true);
                    
                    let processed = state.deep_processed + state.shallow_processed;
                    let total = state.total_files || 0;
                    
                    if (total > 0) {
                        let fraction = processed / total;
                        if (fraction > 1.0) fraction = 1.0;
                        progressBar.set_fraction(fraction);
                        progressRow.set_title(`Indexing: ${Math.round(fraction * 100)}%`);
                        progressRow.set_subtitle(`Processed: ${processed} / ${total} (Deep: ${state.deep_processed}, Shallow: ${state.shallow_processed})`);
                    } else {
                        progressBar.pulse();
                        progressRow.set_title('Scanning Filesystem...');
                        progressRow.set_subtitle('Calculating missing and modified files...');
                    }
                } else {
                    progressBox.set_visible(false);
                    progressBar.set_fraction(0.0);
                    progressRow.set_title('Idle');
                    progressRow.set_subtitle('System is resting or listening for real-time changes.');
                }
            }
        });
        return GLib.SOURCE_CONTINUE;
    });

    const scopeGroup = new Adw.PreferencesGroup({ title: 'Indexing Scope' });
    
    const fullSysRow = new Adw.SwitchRow({
        title: 'Full Home Directory Indexation',
        subtitle: 'Index all files recursively inside your home folder. Warning: Can be resource intensive.'
    });
    settings.bind('index-full-system', fullSysRow, 'active', Gio.SettingsBindFlags.DEFAULT);
    scopeGroup.add(fullSysRow);

    const depthRow = new Adw.SpinRow({
        title: 'Max Recursion Depth',
        subtitle: 'Requires a daemon restart to apply new kernel watches.',
        adjustment: new Gtk.Adjustment({
             lower: 1,
             upper: 15,
             step_increment: 1,
             value: settings.get_int('index-max-depth')
         })
    });
    settings.bind('index-max-depth', depthRow.adjustment, 'value', Gio.SettingsBindFlags.DEFAULT);
    
    depthRow.adjustment.connect('value-changed', () => {
        if (depthRow.adjustment.value > 3) {
            depthRow.set_subtitle('High depth detected. You MUST manually increase fs.inotify.max_user_watches in your OS.');
        } else {
            depthRow.set_subtitle('Requires a daemon restart to apply new kernel watches.');
        }
    });
    
    if (settings.get_int('index-max-depth') > 3) {
        depthRow.set_subtitle('High depth detected. You MUST manually increase fs.inotify.max_user_watches in your OS.');
    }
    
    scopeGroup.add(depthRow);
    page.add(scopeGroup);

    const pathGroup = new Adw.PreferencesGroup({ 
         title: 'Specific Target Directories',
        description: 'Directories to recursively index when Full Home Indexation is disabled.'
    });
    
    let pathRows = [];
    
    const updatePaths = () => {
        pathRows.forEach(row => pathGroup.remove(row));
        pathRows = [];
        
        let paths = settings.get_strv('index-paths') || [];
        for (let p of paths) {
            let row = new Adw.ActionRow({ title: p });
            let delBtn = new Gtk.Button({
                icon_name: 'user-trash-symbolic',
                valign: Gtk.Align.CENTER,
                margin_end: 8
            });
            delBtn.add_css_class('destructive-action');
            delBtn.connect('clicked', () => {
                let newPaths = settings.get_strv('index-paths').filter(x => x !== p);
                settings.set_strv('index-paths', newPaths);
                updatePaths();
            });
            row.add_suffix(delBtn);
            pathGroup.add(row);
            pathRows.push(row);
        }
    };

    const addPathRow = new Adw.ActionRow({ title: 'Add Directory...' });
    const addPathBtn = new Gtk.Button({
        icon_name: 'list-add-symbolic',
        valign: Gtk.Align.CENTER,
        margin_end: 8
    });
    addPathBtn.add_css_class('suggested-action');
    
    addPathBtn.connect('clicked', () => {
        let dialog = new Gtk.FileDialog({ title: 'Select Directory to Index' });
        dialog.select_folder(window, null, (dlg, res) => {
            try {
                let file = dlg.select_folder_finish(res);
                if (file) {
                    let path = file.get_path();
                    let home = GLib.get_home_dir();
                    if (path.startsWith(home)) {
                        path = '~' + path.substring(home.length);
                    }
                    
                    let currentPaths = settings.get_strv('index-paths') || [];
                    if (!currentPaths.includes(path)) {
                        currentPaths.push(path);
                        settings.set_strv('index-paths', currentPaths);
                        updatePaths();
                    }
                }
            } catch (e) {
            }
        });
    });
    
    addPathRow.add_suffix(addPathBtn);
    pathGroup.add(addPathRow);
    updatePaths();

    let fullSysChangedId = settings.connect('changed::index-full-system', () => {
        pathGroup.set_sensitive(!settings.get_boolean('index-full-system'));
    });
    pathGroup.set_sensitive(!settings.get_boolean('index-full-system'));
    
    page.add(pathGroup);

    const blacklistGroup = new Adw.PreferencesGroup({ 
         title: 'Blacklisted Names',
        description: 'Folder or file names that will be explicitly ignored during indexing (e.g. node_modules, .git).'
    });
    
    let blacklistRows = [];
    const updateBlacklist = () => {
        blacklistRows.forEach(row => blacklistGroup.remove(row));
        blacklistRows = [];
        
        let items = settings.get_strv('index-blacklist') || [];
        for (let item of items) {
            let row = new Adw.ActionRow({ title: item });
            let delBtn = new Gtk.Button({
                icon_name: 'user-trash-symbolic',
                valign: Gtk.Align.CENTER,
                margin_end: 8
            });
            delBtn.add_css_class('destructive-action');
            delBtn.connect('clicked', () => {
                let newItems = settings.get_strv('index-blacklist').filter(x => x !== item);
                settings.set_strv('index-blacklist', newItems);
                updateBlacklist();
            });
            row.add_suffix(delBtn);
            blacklistGroup.add(row);
            blacklistRows.push(row);
        }
    };

    const addBlacklistRow = new Adw.EntryRow({ 
         title: 'Add new ignore rule...',
        show_apply_button: true 
    });
    addBlacklistRow.connect('apply', () => {
        let text = addBlacklistRow.get_text().trim();
        if (text) {
            let items = settings.get_strv('index-blacklist') || [];
            if (!items.includes(text)) {
                items.push(text);
                settings.set_strv('index-blacklist', items);
                updateBlacklist();
            }
            addBlacklistRow.set_text('');
        }
    });

    blacklistGroup.add(addBlacklistRow);
    updateBlacklist();
    
    page.add(blacklistGroup);

    const maintenanceGroup = new Adw.PreferencesGroup({ 
         title: 'Database Maintenance',
        description: 'Advanced options for managing the vector index.'
    });

    const statsRow = new Adw.ActionRow({
        title: 'Index Statistics',
        subtitle: 'Connecting to background service...'
    });

    const refreshBtn = new Gtk.Button({
        icon_name: 'view-refresh-symbolic',
        valign: Gtk.Align.CENTER,
        margin_end: 8,
        tooltip_text: 'Refresh Database Stats'
    });
    refreshBtn.add_css_class('flat');
    statsRow.add_suffix(refreshBtn);
    
    const loadStats = () => {
        statsRow.set_subtitle('Fetching statistics...');
        sendDaemonCommand({ action: 'get_db_stats' }, (data) => {
            if (data.status === 'db_stats') {
                let records = data.records;
                let bytes = data.size_bytes;
                
                let sizes = ['Bytes', 'KB', 'MB', 'GB', 'TB'];
                let i = 0;
                let size = bytes;
                while (size >= 1024 && i < sizes.length - 1) {
                    size /= 1024;
                    i++;
                }
                let sizeStr = size.toFixed(2) + ' ' + sizes[i];
                
                statsRow.set_subtitle(`${records} items indexed     ${sizeStr} on disk`);
            } else if (data.status === 'error') {
                statsRow.set_subtitle('Service offline. Please start the background daemon.');
            }
        });
    };

    refreshBtn.connect('clicked', loadStats);
    maintenanceGroup.add(statsRow);

    const reindexRow = new Adw.ActionRow({
        title: 'Force Full Re-index',
        subtitle: 'Reset internal timestamps to force the background daemon to deep-scan all files again.'
    });

    const reindexBtn = new Gtk.Button({
        label: 'Re-index',
        valign: Gtk.Align.CENTER,
        margin_end: 8
    });
    reindexBtn.add_css_class('destructive-action');
    
    reindexBtn.connect('clicked', () => {
        reindexBtn.set_sensitive(false);
        reindexBtn.set_label('Triggered...');
        
        sendDaemonCommand({ action: 'reindex' }, null);
        
        safeTimeout(3000, () => {
            reindexBtn.set_sensitive(true);
            reindexBtn.set_label('Re-index');
            loadStats(); 
        });
    });

    reindexRow.add_suffix(reindexBtn);
    maintenanceGroup.add(reindexRow);
    page.add(maintenanceGroup);

    updateServiceUI();
    loadStats();

    window.connect('close-request', () => {
        if (healthCheckId > 0) {
            GLib.source_remove(healthCheckId);
            healthCheckId = 0;
        }
        for (let t of _timeoutIds) {
            if (t > 0) GLib.source_remove(t);
        }
        _timeoutIds = [];
        if (fullSysChangedId > 0) {
            settings.disconnect(fullSysChangedId);
            fullSysChangedId = 0;
        }
    });

    return page;
}