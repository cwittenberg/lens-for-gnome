// gnome-extension/prefs_ai.js
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
        this.statusGroup = new Adw.PreferencesGroup({ title: 'Daemon Status' });

        this.statusRow = new Adw.ActionRow({ title: '🟡 Connecting...', subtitle: 'Pinging IPC socket...' });
        
        this.spinner = new Gtk.Spinner({
            valign: Gtk.Align.CENTER,
            margin_end: 12
        });
        this.statusRow.add_prefix(this.spinner);
        
        this.statusGroup.add(this.statusRow);

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
                this.statusRow.set_title('🟢 Daemon Online');
                
                let outputStream = connection.get_output_stream();
                let inputStream = new Gio.DataInputStream({ base_stream: connection.get_input_stream() });
                
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
                    this.statusRow.set_title('🔴 Daemon Offline');
                    this.statusRow.set_subtitle('Is the rust background service running?');
                    this.hwStatusRow.set_subtitle('Daemon unreachable.');
                    this.spinner.stop();
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
                if (lineData && lineData[0]) {
                    onMessage(JSON.parse(lineData[0]));
                    // Continue reading if the daemon is streaming chunks (like cURL progress)
                    this._readLoop(inputStream, cancellable, onMessage, connection);
                } else {
                    // Daemon closed stream gracefully after fulfilling the request
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

    requestConfig() {
        this.spinner.start();
        this.statusRow.set_subtitle('Synchronizing...');
        
        this._sendPayload({ action: 'get_config' }, (data) => {
            if (data.status === 'config_data') {
                this.spinner.stop();
                this.statusRow.set_subtitle('Ready for integration.');
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

    switchModel(modelId) {
        for (let btn of this.switchButtons) {
            btn.set_sensitive(false);
        }
        
        this.spinner.start();
        this.statusRow.set_title('⚙️ Executing Model Hotswap...');
        this.statusRow.set_subtitle('This may take several minutes if a download is required. Do not close this window.');
        
        this._sendPayload({ action: 'update_config', key: 'active_model', value: modelId }, (data) => {
            if (data.status === 'processing') {
                this.statusRow.set_subtitle(data.message);
            } else if (data.status === 'done') {
                this.statusRow.set_title('🟢 Daemon Online');
                this.statusRow.set_subtitle(data.message || 'Ready for integration.');
                this.requestConfig();
            } else if (data.status === 'error') {
                this.spinner.stop();
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
            let row = new Adw.ActionRow({
                title: info.name,
                subtitle: `${info.description}\nSize: ${info.size_gb}GB | RAM Required: ${info.ram_required_gb}GB | Params: ${info.parameters}`,
            });
            row.set_subtitle_lines(3);

            let button = new Gtk.Button({
                valign: Gtk.Align.CENTER,
            });

            if (id === activeModelId) {
                button.set_label('Active');
                button.set_sensitive(false);
                button.add_css_class('suggested-action');
            } else {
                button.set_label('Switch');
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