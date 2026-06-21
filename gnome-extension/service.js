// gnome-extension/service.js
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';

export default class ServiceClient {
    constructor() {
        this._cancellable = null;
        this._socketClient = null;
        this._connection = null;
        this._inputStream = null;
        this._outputStream = null;
    }

    sendPayload(payloadObj, callbacks) {
        this.cancel();
        this._cancellable = new Gio.Cancellable();
        this._socketClient = new Gio.SocketClient();
        this.callbacks = callbacks || {};

        let socketPath = GLib.get_home_dir() + '/.local/state/gnome-lens/gnome_lens.sock';
        let address = Gio.UnixSocketAddress.new(socketPath);

        this._socketClient.connect_async(address, this._cancellable, (client, res) => {
            try {
                this._connection = client.connect_finish(res);
            } catch (error) {
                if (!error.matches(Gio.IOErrorEnum, Gio.IOErrorEnum.CANCELLED)) {
                    if (this.callbacks.onOffline) this.callbacks.onOffline();
                }
                return;
            }

            this._outputStream = this._connection.get_output_stream();
            this._inputStream = new Gio.DataInputStream({ base_stream: this._connection.get_input_stream() });
            this._inputStream.set_newline_type(Gio.DataStreamNewlineType.ANY);

            let payload = JSON.stringify(payloadObj) + '\n';

            this._outputStream.write_all_async(payload, GLib.PRIORITY_DEFAULT, this._cancellable, (stream, writeRes) => {
                try {
                    stream.write_all_finish(writeRes);
                } catch (error) {
                    if (this.callbacks.onError) this.callbacks.onError(error);
                    return;
                }
                this._readStream();
            });
        });
    }

    search(query, callbacks) {
        this.sendPayload({ query: query }, callbacks);
    }

    _readStream() {
        if (!this._inputStream || !this._cancellable || this._cancellable.is_cancelled()) {
            return;
        }

        this._inputStream.read_line_async(GLib.PRIORITY_DEFAULT, this._cancellable, (stream, res) => {
            let lineData;
            try {
                lineData = stream.read_line_finish_utf8(res);
            } catch (error) {
                return;
            }

            if (!lineData || !lineData[0]) {
                return;
            }

            let text = lineData[0].trim();
            if (text.length > 0) {
                try {
                    let parsed = JSON.parse(text);
                    if (this.callbacks.onMessage) this.callbacks.onMessage(parsed);
                } catch (error) {
                    console.warn(`[Gnome Lens] Ignoring invalid service JSON: ${error}`);
                }
            }

            this._readStream();
        });
    }

    cancel() {
        if (this._outputStream) {
            try {
                let payload = JSON.stringify({ action: 'cancel' }) + '\n';
                this._outputStream.write_all(payload, null);
            } catch(e) {}
        }

        if (this._cancellable) {
            this._cancellable.cancel();
            this._cancellable = null;
        }
        if (this._inputStream) {
            this._inputStream.close_async(GLib.PRIORITY_DEFAULT, null, () => {});
            this._inputStream = null;
        }
        if (this._outputStream) {
            this._outputStream.close_async(GLib.PRIORITY_DEFAULT, null, () => {});
            this._outputStream = null;
        }
        if (this._connection) {
            this._connection.close_async(GLib.PRIORITY_DEFAULT, null, () => {});
            this._connection = null;
        }
        if (this._socketClient) {
            this._socketClient = null;
        }
    }
}