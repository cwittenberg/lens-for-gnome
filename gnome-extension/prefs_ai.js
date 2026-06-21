import Adw from 'gi://Adw';
import Gio from 'gi://Gio';
import Gtk from 'gi://Gtk';
import GLib from 'gi://GLib';

class AIEngineManager {
    constructor(page) {
        this.page = page;
        this._cancellables = [];
        this.modelRows = [];
        this.switchButtons = [];

        this.buildUI();
        this.requestConfig();
        this.requestHardwareStatus();
    }

    buildUI() {
        this.statusGroup = new Adw.PreferencesGroup({ title: 'Service Status' });

        this.statusRow = new Adw.ActionRow({ title: '🟡 Connecting...', subtitle: 'Pinging IPC socket...' });
        
        this.spinner = new Gtk.Spinner({
            valign: Gtk.Align.CENTER,
            visible: false
        });
        this.statusRow.add_suffix(this.spinner);
        
        this.statusGroup.add(this.statusRow);

        this.progressBox = new Gtk.Box({
            orientation: Gtk.Orientation.HORIZONTAL,
            spacing: 8,
            margin_top: 4,
            margin_bottom: 12,
            margin_start: 12,
            margin_end: 12,
            visible: false
        });

        this.progressBar = new Gtk.ProgressBar({
            show_text: true,
            hexpand: true,
            valign: Gtk.Align.CENTER
        });

        this.cancelButton = new Gtk.Button({
            icon_name: 'process-stop-symbolic',
            valign: Gtk.Align.CENTER,
            tooltip_text: 'Cancel Download'
        });
        this.cancelButton.add_css_class('destructive-action');
        this.cancelButton.connect('clicked', () => this.cancelActiveOperations());

        this.progressBox.append(this.progressBar);
        this.progressBox.append(this.cancelButton);
        this.statusGroup.add(this.progressBox);

        this.hwStatusRow = new Adw.ActionRow({ 
            title: '🖥️ Hardware Optimization', 
            subtitle: 'Detecting...' 
        });
        this.statusGroup.add(this.hwStatusRow);

        this.page.add(this.statusGroup);

        this.modelGroup = new Adw.PreferencesGroup({ 
             title: 'Available AI Models',
             description: 'Fetching configurations...'
         });
        this.page.add(this.modelGroup);
    }

    _sendPayload(payloadObj, onMessage) {
        let cancellable = new Gio.Cancellable();
        this._cancellables.push(cancellable);

        let socketClient = new Gio.SocketClient();
        let socketPath = GLib.get_home_dir() + '/.local/state/gnome-lens/gnome_lens.sock';
        let address = Gio.UnixSocketAddress.new(socketPath);

        socketClient.connect_async(address, cancellable, (client, res) => {
            try {
                let connection = client.connect_finish(res);
                this.statusRow.set_title('🟢 Service Online');
                
                let outputStream = connection.get_output_stream();
                let inputStream = new Gio.DataInputStream({ base_stream: connection.get_input_stream() });
                inputStream.set_newline_type(Gio.DataStreamNewlineType.ANY);
                
                let payloadStr = JSON.stringify(payloadObj) + '\n';
                
                outputStream.write_all_async(payloadStr, GLib.PRIORITY_DEFAULT, cancellable, (stream, writeRes) => {
                    try {
                        stream.write_all_finish(writeRes);
                        this._readLoop(inputStream, cancellable, onMessage, connection);
                    } catch (e) {
                        this._cleanupConnection(inputStream, outputStream, connection);
                    }
                });
            } catch (e) {
                if (!cancellable.is_cancelled()) {
                    this.statusRow.set_title('🔴 Service Offline');
                    this.statusRow.set_subtitle('Is the rust background service running?');
                    this.hwStatusRow.set_subtitle('Service unreachable.');
                    this.spinner.stop();
                    this.spinner.set_visible(false);
                }
            }
        });
    }

    _readLoop(inputStream, cancellable, onMessage, connection) {
        if (cancellable.is_cancelled()) {
            this._cleanupConnection(inputStream, null, connection);
            return;
        }

        inputStream.read_line_async(GLib.PRIORITY_DEFAULT, cancellable, (stream, res) => {
            try {
                let lineData = stream.read_line_finish_utf8(res);
                if (lineData && lineData[0] !== null) {
                    let text = lineData[0].trim();
                    if (text.length > 0) {
                        try {
                            onMessage(JSON.parse(text));
                        } catch (err) {
                            console.warn('[Gnome Lens] Invalid JSON in stream:', text);
                        }
                    }
                    this._readLoop(inputStream, cancellable, onMessage, connection);
                } else {
                    this._cleanupConnection(inputStream, null, connection);
                }
            } catch (error) {
                this._cleanupConnection(inputStream, null, connection);
            }
        });
    }

    _cleanupConnection(inputStream, outputStream, connection) {
        if (inputStream) {
            try { inputStream.close_async(GLib.PRIORITY_DEFAULT, null, () => {}); } catch(e) {}
        }
        if (outputStream) {
            try { outputStream.close_async(GLib.PRIORITY_DEFAULT, null, () => {}); } catch(e) {}
        }
        if (connection) {
            try { connection.close_async(GLib.PRIORITY_DEFAULT, null, () => {}); } catch(e) {}
        }
    }

    cancelActiveOperations() {
        for (let c of this._cancellables) {
            c.cancel();
        }
        this._cancellables = [];
        this.spinner.stop();
        this.spinner.set_visible(false);
        this.progressBox.set_visible(false);
        this.statusRow.set_title('🟡 Operation Cancelled');
        this.statusRow.set_subtitle('Connecting...');

        // Give the daemon a moment to process the disconnected socket and kill the curl thread
        GLib.timeout_add(GLib.PRIORITY_DEFAULT, 1000, () => {
            this.requestConfig();
            return GLib.SOURCE_REMOVE;
        });
    }

    requestConfig() {
        this.spinner.set_visible(true);
        this.spinner.start();
        this.statusRow.set_subtitle('Synchronizing...');
        
        this._sendPayload({ action: 'get_config' }, (data) => {
            if (data.status === 'config_data') {
                this.spinner.stop();
                this.spinner.set_visible(false);
                this.statusRow.set_subtitle('Ready.');
                this._renderModels(data.data);
            }
        });
    }

    requestHardwareStatus() {
        this._sendPayload({ action: 'get_hardware_status' }, (data) => {
            if (data.status === 'hardware_data') {
                let hw = data.data;
                if (hw.is_hardware_dedicated && hw.acceleration_type !== 'CPU') {
                    this.hwStatusRow.set_title(`⚡ Active: ${hw.acceleration_type} Acceleration`);
                    this.hwStatusRow.set_subtitle(`${hw.device_name} (via ${hw.api})`);
                } else {
                    this.hwStatusRow.set_title('💻 Active: CPU Mode (Software)');
                    this.hwStatusRow.set_subtitle('No dedicated hardware acceleration detected.');
                }
            }
        });
    }

    deleteModel(modelId) {
        for (let btn of this.switchButtons) {
            btn.set_sensitive(false);
        }
        this.spinner.set_visible(true);
        this.spinner.start();
        this.statusRow.set_title('⚙️ Deleting Model...');
        this.statusRow.set_subtitle('Removing model files from disk.');

        this._sendPayload({ action: 'delete_model', model_id: modelId }, (data) => {
            if (data.status === 'done') {
                this.statusRow.set_title('🟢 Service Online');
                this.statusRow.set_subtitle('Model removed successfully.');
                this.requestConfig();
            } else if (data.status === 'error') {
                this.spinner.stop();
                this.spinner.set_visible(false);
                this.statusRow.set_title('🔴 Engine Error');
                this.statusRow.set_subtitle(data.message);
                this.requestConfig();
            }
        });
    }

    switchModel(modelId) {
        for (let btn of this.switchButtons) {
            btn.set_sensitive(false);
        }
        
        this.spinner.set_visible(true);
        this.spinner.start();
        this.progressBox.set_visible(false);
        this.progressBar.set_fraction(0.0);
        this.statusRow.set_title('⚙️ Executing Model Hotswap...');
        this.statusRow.set_subtitle('This may take several minutes if a download is required. Do not close this window.');
        
        this._sendPayload({ action: 'update_config', key: 'active_model', value: modelId }, (data) => {
            if (data.status === 'processing') {
                this.statusRow.set_subtitle(data.message);
            } else if (data.status === 'downloading') {
                this.statusRow.set_subtitle('Downloading Model...');
                this.progressBox.set_visible(true);
                let fraction = data.progress / 100.0;
                if (fraction >= 0.0 && fraction <= 1.0) {
                    this.progressBar.set_fraction(fraction);
                }
            } else if (data.status === 'done') {
                this.statusRow.set_title('🟢 Service Online');
                this.statusRow.set_subtitle(data.message || 'Ready.');
                this.progressBox.set_visible(false);
                this.requestConfig();
            } else if (data.status === 'error') {
                this.spinner.stop();
                this.spinner.set_visible(false);
                this.progressBox.set_visible(false);
                this.statusRow.set_title('🔴 Engine Error');
                this.statusRow.set_subtitle(data.message);
                this.requestConfig();
            }
        });
    }

    _renderModels(configData) {
        let activeModelId = configData.active_model;
        let models = configData.models;
        
        this.modelGroup.set_description('Select the local AI model to power Gnome Lens. Larger models provide better reasoning but require more system RAM.');
        
        for (let r of this.modelRows) {
            this.modelGroup.remove(r);
        }
        
        this.modelRows = [];
        this.switchButtons = [];

        for (let [id, info] of Object.entries(models)) {
            let isInstalled = info.is_installed === true;
            let installedLabel = isInstalled ? ' (Installed)' : '';

            let row = new Adw.ActionRow({
                title: info.name + installedLabel,
                subtitle: `${info.description}\nSize: ${info.size_gb}GB | RAM Required: ${info.ram_required_gb}GB | Params: ${info.parameters}`,
            });
            row.set_subtitle_lines(3);

            if (isInstalled && id !== activeModelId) {
                let delButton = new Gtk.Button({
                    valign: Gtk.Align.CENTER,
                    icon_name: 'user-trash-symbolic',
                    margin_end: 6,
                    tooltip_text: 'Delete Model'
                });
                delButton.add_css_class('destructive-action');
                delButton.connect('clicked', () => {
                    this.deleteModel(id);
                });
                row.add_suffix(delButton);
                this.switchButtons.push(delButton);
            }

            let button = new Gtk.Button({
                valign: Gtk.Align.CENTER,
            });

            if (id === activeModelId) {
                button.set_label('Active');
                button.set_sensitive(false);
                button.add_css_class('suggested-action');
            } else {
                button.set_label(isInstalled ? 'Switch' : 'Download & Switch');
                button.connect('clicked', () => {
                    this.switchModel(id);
                });
                this.switchButtons.push(button);
            }

            row.add_suffix(button);
            this.modelGroup.add(row);
            this.modelRows.push(row);
        }
    }

    destroy() {
        for (let c of this._cancellables) {
            c.cancel();
        }
    }
}

export function buildAIPage(window) {
    const page = new Adw.PreferencesPage({
         title: 'AI Engine',
         icon_name: 'applications-engineering-symbolic'
     });

    let aiManager = new AIEngineManager(page);

    window.connect('close-request', () => {
        aiManager.destroy();
    });

    return page;
}