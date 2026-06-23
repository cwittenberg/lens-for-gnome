// gnome-extension/prefs_index.js
import Adw from 'gi://Adw';
import Gtk from 'gi://Gtk';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';

// Advanced wrapper for firing UI commands into the Rust IPC Socket and asynchronously receiving responses
function sendDaemonCommand(payloadObj, onMessage) {
    let cancellable = new Gio.Cancellable();
    let socketClient = new Gio.SocketClient();
    let socketPath = GLib.get_home_dir() + '/.local/state/gnome-lens/gnome_lens.sock';
    let address = Gio.UnixSocketAddress.new(socketPath);

    socketClient.connect_async(address, cancellable, (client, res) => {
        try {
            let connection = client.connect_finish(res);
            let outputStream = connection.get_output_stream();
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
                                    }
                                } catch (e) {}
                            });
                        };
                        readLoop();
                    }
                } catch (e) {
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

// Helper to find the binary even if GNOME Shell is missing user PATH variables
// Thinks outside the box by automatically falling back to common development paths.
function getDaemonExecPath() {
    let execPath = GLib.find_program_in_path('gnome-lens');
    if (execPath) return execPath;

    let home = GLib.get_home_dir();
    
    // 1. Check standard system and cargo locations, plus explicit hot-reload dev paths
    let standardPaths = [
        home + '/.cargo/bin/gnome-lens',
        home + '/.local/bin/gnome-lens',
        home + '/Development/extensions/gnome-lens/target/release/gnome-lens',
        home + '/Development/extensions/gnome-lens/target/debug/gnome-lens'
    ];

    for (let p of standardPaths) {
        if (GLib.file_test(p, GLib.FileTest.EXISTS)) return p;
    }

    // 2. Think outside the box: Dynamic search fallback
    // In case the project was cloned into a different dev directory, execute a quick sync search
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

    return 'gnome-lens'; // Absolute fallback
}

export function buildIndexPage(settings, window) {
    const page = new Adw.PreferencesPage({ 
        title: 'Indexation', 
        icon_name: 'folder-saved-search-symbolic' 
    });

    let isProcessing = false;

    // ==========================================
    // 1. SERVICE DAEMON CONTROLLER GROUP
    // ==========================================
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

    // Control Buttons
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

    // Native IPC and Subprocess integration (EGO-Compliant)
    const updateServiceUI = (silent = false) => {
        if (isProcessing) return; // Prevent background pings from overriding active UI flows

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
                statusRow.set_title('🔴 Service Status: Stopped');
                statusRow.set_subtitle('The background ingestion engine is inactive.');
                startBtn.set_sensitive(true);
                stopBtn.set_sensitive(false);
                restartBtn.set_sensitive(false);
            } else {
                statusRow.set_title('🟢 Service Status: Running');
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
        statusRow.set_title('🟡 Service Status: Starting...');
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
            
            GLib.timeout_add(GLib.PRIORITY_DEFAULT, 1000, () => {
                isProcessing = false;
                updateServiceUI();
                loadStats();
                return GLib.SOURCE_REMOVE;
            });
        } catch (e) {
            isProcessing = false;
            spinner.stop();
            spinner.set_visible(false);
            statusRow.set_title('🔴 Launch Failed');
            statusRow.set_subtitle(e.message);
            startBtn.set_sensitive(true);
        }
    });

    stopBtn.connect('clicked', () => {
        isProcessing = true;
        startBtn.set_sensitive(false);
        stopBtn.set_sensitive(false);
        restartBtn.set_sensitive(false);
        statusRow.set_title('🟡 Service Status: Stopping...');
        statusRow.set_subtitle('Shutting down gracefully...');
        spinner.set_visible(true);
        spinner.start();

        sendDaemonCommand({ action: 'shutdown' }, () => {});
        
        GLib.timeout_add(GLib.PRIORITY_DEFAULT, 1000, () => {
            isProcessing = false;
            updateServiceUI();
            return GLib.SOURCE_REMOVE;
        });
    });

    restartBtn.connect('clicked', () => {
        isProcessing = true;
        startBtn.set_sensitive(false);
        stopBtn.set_sensitive(false);
        restartBtn.set_sensitive(false);
        statusRow.set_title('🟡 Service Status: Restarting...');
        statusRow.set_subtitle('Cycling daemon processes...');
        spinner.set_visible(true);
        spinner.start();

        sendDaemonCommand({ action: 'shutdown' }, () => {});
        
        // Wait longer (1.5s) to ensure the previous UNIX socket is completely flushed and unlinked
        GLib.timeout_add(GLib.PRIORITY_DEFAULT, 1500, () => {
            try {
                let execPath = getDaemonExecPath();
                let proc = new Gio.Subprocess({
                    argv: [execPath],
                    flags: Gio.SubprocessFlags.NONE
                });
                proc.init(null);
            } catch (e) {
                console.warn("Restart failed to launch binary:", e);
                statusRow.set_title('🔴 Launch Failed');
                statusRow.set_subtitle(e.message);
                spinner.stop();
                spinner.set_visible(false);
                isProcessing = false;
                startBtn.set_sensitive(true);
                return GLib.SOURCE_REMOVE;
            }

            GLib.timeout_add(GLib.PRIORITY_DEFAULT, 1000, () => {
                isProcessing = false;
                updateServiceUI();
                loadStats();
                return GLib.SOURCE_REMOVE;
            });
            return GLib.SOURCE_REMOVE;
        });
    });

    statusRow.add_suffix(restartBtn);
    statusRow.add_suffix(startBtn);
    statusRow.add_suffix(stopBtn);
    serviceGroup.add(statusRow);
    page.add(serviceGroup);

    // Set up continuous background health check every 3 seconds
    let healthCheckId = GLib.timeout_add_seconds(GLib.PRIORITY_DEFAULT, 3, () => {
        updateServiceUI(true);
        return GLib.SOURCE_CONTINUE;
    });

    window.connect('close-request', () => {
        if (healthCheckId > 0) {
            GLib.source_remove(healthCheckId);
            healthCheckId = 0;
        }
    });

    // ==========================================
    // 2. SCOPE GROUP (Full System & Depth)
    // ==========================================
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

    // ==========================================
    // 3. TARGET PATHS GROUP
    // ==========================================
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
                // User cancelled dialog
            }
        });
    });
    addPathRow.add_suffix(addPathBtn);
    pathGroup.add(addPathRow);
    updatePaths();

    settings.connect('changed::index-full-system', () => {
        pathGroup.set_sensitive(!settings.get_boolean('index-full-system'));
    });
    pathGroup.set_sensitive(!settings.get_boolean('index-full-system'));
    
    page.add(pathGroup);

    // ==========================================
    // 4. BLACKLIST GROUP
    // ==========================================
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

    // ==========================================
    // 5. DATABASE MAINTENANCE GROUP
    // ==========================================
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
                
                statsRow.set_subtitle(`${records} items indexed  •  ${sizeStr} on disk`);
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
        
        GLib.timeout_add(GLib.PRIORITY_DEFAULT, 3000, () => {
            reindexBtn.set_sensitive(true);
            reindexBtn.set_label('Re-index');
            loadStats(); 
            return GLib.SOURCE_REMOVE;
        });
    });

    reindexRow.add_suffix(reindexBtn);
    maintenanceGroup.add(reindexRow);
    page.add(maintenanceGroup);

    // Initial Bootstrap
    updateServiceUI();
    loadStats();

    return page;
}