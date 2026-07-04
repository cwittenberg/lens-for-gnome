import Clutter from 'gi://Clutter';
import Cogl from 'gi://Cogl';
import St from 'gi://St';
import GObject from 'gi://GObject';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';

let Gst = null;
let GstLoaded = false;
let GstLoadFailed = false;

const HISTORY_FILE = GLib.build_filenamev([GLib.get_user_cache_dir(), 'lens-for-gnome-playback.json']);
export const PlaybackHistory = new Map();
let _historyLoadPromise = null;

function ensureHistoryLoaded() {
    if (_historyLoadPromise) return _historyLoadPromise;
    
    _historyLoadPromise = new Promise((resolve) => {
        let file = Gio.File.new_for_path(HISTORY_FILE);
        
        file.load_contents_async(null, (f, res) => {
            try {
                let [success, contents] = f.load_contents_finish(res);
                if (success) {
                    let data = JSON.parse(new TextDecoder().decode(contents));
                    for (let [k, v] of Object.entries(data)) {
                        PlaybackHistory.set(k, v);
                    }
                }
            } catch (e) {
                console.debug(`[Lens for GNOME] History load note: ${e.message}`);
            }
            resolve();
        });
    });
    
    return _historyLoadPromise;
}

function savePlaybackHistoryAsync() {
    if (PlaybackHistory.size > 50) {
        let keys = Array.from(PlaybackHistory.keys());
        let keysToRemove = keys.slice(0, PlaybackHistory.size - 50);
        for (let k of keysToRemove) PlaybackHistory.delete(k);
    }
    let file = Gio.File.new_for_path(HISTORY_FILE);
    let obj = Object.fromEntries(PlaybackHistory);
    let bytes = new GLib.Bytes(new TextEncoder().encode(JSON.stringify(obj)));
    
    file.replace_contents_bytes_async(
        bytes,
        null,
        false,
        Gio.FileCreateFlags.REPLACE_DESTINATION,
        null,
        (f, res) => {
            try {
                f.replace_contents_finish(res);
            } catch(e) {
                console.warn(`[Lens for GNOME] Failed to write playback history to disk: ${e.message}`);
            }
        }
    );
}

async function ensureGst() {
    if (GstLoaded) return true;
    if (GstLoadFailed) return false;
    try {
        let gi = await import('gi://Gst');
        Gst = gi.default;
        Gst.init(null);
        await import('gi://GstApp');
        GstLoaded = true;
        return true;
    } catch (e) {
        console.warn(`[Lens for GNOME] GStreamer unavailable. Video playback will use fallback: ${e.message}`);
        GstLoadFailed = true;
        return false;
    }
}

function formatTime(nanoseconds) {
    if (!nanoseconds || nanoseconds < 0) return '00:00';
    let totalSeconds = Math.floor(nanoseconds / 1000000000);
    let mins = Math.floor(totalSeconds / 60);
    let secs = totalSeconds % 60;
    return `${mins.toString().padStart(2, '0')}:${secs.toString().padStart(2, '0')}`;
}

export const GnomeLensVideoControls = GObject.registerClass({
    GTypeName: 'GnomeLensVideoControls'
}, class GnomeLensVideoControls extends St.BoxLayout {
    _init(player) {
        super._init({
            style_class: 'lens-video-hud',
            vertical: true,
            reactive: true
        });
        this._player = player;
        this._isDraggingScrub = false;
        this._isDraggingVolume = false;
        this._buildControlsUI();
    }

    _buildControlsUI() {
        this._scrubBar = new St.BoxLayout({
            style_class: 'lens-slider-track',
            vertical: false,
            x_expand: true,
            reactive: true,
            height: 12
        });
        
        this._scrubFill = new St.Widget({
            style_class: 'lens-slider-fill',
            width: 0,
            x_align: Clutter.ActorAlign.START
        });
        this._scrubBar.add_child(this._scrubFill);
        this.add_child(this._scrubBar);

        this._scrubBar.connectObject('button-press-event', (actor, event) => {
            this._isDraggingScrub = true;
            this._processScrubLocation(event);
            return Clutter.EVENT_STOP;
        }, this);

        this._scrubBar.connectObject('motion-event', (actor, event) => {
            if (this._isDraggingScrub) {
                this._processScrubLocation(event);
                return Clutter.EVENT_STOP;
            }
            return Clutter.EVENT_PROPAGATE;
        }, this);

        this._scrubBar.connectObject('button-release-event', () => {
            this._isDraggingScrub = false;
            return Clutter.EVENT_STOP;
        }, this);

        let toolRow = new St.BoxLayout({
            vertical: false,
            x_expand: true,
            y_align: Clutter.ActorAlign.CENTER
        });

        this._timeLabel = new St.Label({
            style_class: 'lens-time-label',
            text: '00:00 / 00:00',
            y_align: Clutter.ActorAlign.CENTER
        });
        toolRow.add_child(this._timeLabel);

        let centerSpacer = new St.Widget({ x_expand: true });
        toolRow.add_child(centerSpacer);

        this._volumeBox = new St.BoxLayout({
            style_class: 'lens-volume-box',
            vertical: false,
            y_align: Clutter.ActorAlign.CENTER
        });

        this._muteBtn = new St.Button({
            style_class: 'lens-video-control-btn',
            child: new St.Icon({ icon_name: 'audio-volume-high-symbolic', icon_size: 14 }),
            y_align: Clutter.ActorAlign.CENTER
        });
        
        this._muteBtn.connectObject('clicked', () => {
            this._player.toggleMute();
            return Clutter.EVENT_STOP;
        }, this);
        this._volumeBox.add_child(this._muteBtn);

        this._volumeTrack = new St.BoxLayout({
            style_class: 'lens-volume-slider',
            width: 70,
            height: 10,
            reactive: true,
            y_align: Clutter.ActorAlign.CENTER
        });
        this._volumeFill = new St.Widget({
            style_class: 'lens-volume-fill',
            width: 50,
            x_align: Clutter.ActorAlign.START
        });
        this._volumeTrack.add_child(this._volumeFill);
        this._volumeBox.add_child(this._volumeTrack);

        toolRow.add_child(this._volumeBox);

        this._volumeTrack.connectObject('button-press-event', (actor, event) => {
            this._isDraggingVolume = true;
            this._processVolumeLocation(event);
            return Clutter.EVENT_STOP;
        }, this);
        this._volumeTrack.connectObject('motion-event', (actor, event) => {
            if (this._isDraggingVolume) {
                this._processVolumeLocation(event);
                return Clutter.EVENT_STOP;
            }
            return Clutter.EVENT_PROPAGATE;
        }, this);
        this._volumeTrack.connectObject('button-release-event', () => {
            this._isDraggingVolume = false;
            return Clutter.EVENT_STOP;
        }, this);

        this.add_child(toolRow);
    }

    _processScrubLocation(event) {
        let [x, y] = event.get_coords();
        let trackAlloc = this._scrubBar.get_allocation_box();
        let trackWidth = trackAlloc.x2 - trackAlloc.x1;
        if (trackWidth <= 0) return;

        let [success, actorX, actorY] = this._scrubBar.transform_stage_point(x, y);
        let percentage = Math.max(0.0, Math.min(1.0, actorX / trackWidth));
        this._player.seekToPercentage(percentage);
    }

    _processVolumeLocation(event) {
        let [x, y] = event.get_coords();
        let trackAlloc = this._volumeTrack.get_allocation_box();
        let trackWidth = trackAlloc.x2 - trackAlloc.x1;
        if (trackWidth <= 0) return;

        let [success, actorX, actorY] = this._volumeTrack.transform_stage_point(x, y);
        let volumeLevel = Math.max(0.0, Math.min(1.0, actorX / trackWidth));
        this._player.setVolumeLevel(volumeLevel);
    }

    updateUIState(positionNs, durationNs, currentVolume, isMuted) {
        let trackAlloc = this._scrubBar.get_allocation_box();
        let trackWidth = trackAlloc.x2 - trackAlloc.x1;
        
        if (trackWidth > 0 && durationNs > 0) {
            let pct = positionNs / durationNs;
            this._scrubFill.set_width(Math.floor(trackWidth * pct));
        }

        this._timeLabel.set_text(`${formatTime(positionNs)} / ${formatTime(durationNs)}`);

        let volAlloc = this._volumeTrack.get_allocation_box();
        let volWidth = volAlloc.x2 - volAlloc.x1;
        if (volWidth > 0) {
            let activeVolPct = isMuted ? 0.0 : currentVolume;
            this._volumeFill.set_width(Math.floor(volWidth * activeVolPct));
        }

        let muteIcon = this._muteBtn.get_child();
        if (muteIcon) {
            if (isMuted || currentVolume === 0) {
                muteIcon.set_icon_name('audio-volume-muted-symbolic');
            } else if (currentVolume < 0.4) {
                muteIcon.set_icon_name('audio-volume-low-symbolic');
            } else if (currentVolume < 0.7) {
                muteIcon.set_icon_name('audio-volume-medium-symbolic');
            } else {
                muteIcon.set_icon_name('audio-volume-high-symbolic');
            }
        }
    }
});

export const GnomeLensVideoPreview = GObject.registerClass({
    GTypeName: 'GnomeLensVideoPreview'
}, class GnomeLensVideoPreview extends St.Widget {
    _init(filepath) {
        super._init({
            name: 'GnomeLensVideoPlayer',
            style_class: 'lens-video-container',
            x_expand: true,
            y_expand: true,
            x_align: Clutter.ActorAlign.FILL,
            y_align: Clutter.ActorAlign.FILL,
            reactive: true
        });

        this._isDestroyed = false;
        this._filepath = filepath;
        this._currentTimeNs = 0;
        this._hasRestoredPosition = false;
        
        this._durationNs = 0;
        this._volumeLevel = 0.8;
        this._isMuted = false;
        
        this._isSeeking = false;
        this._targetSeekNs = 0;

        this._playbackTimerId = 0;
        this._idleRenderId = 0;
        this._hideTimerId = 0;
        
        this._pipeline = null;
        this._sink = null;
        this._busWatchId = 0;

        this._imageContent = null;
        this._contentWidth = 0;
        this._contentHeight = 0;

        this._proc = null;
        this._lastTempFile = null;

        this.set_layout_manager(new Clutter.BinLayout());

        this._imageActor = new Clutter.Actor({
            x_expand: true,
            y_expand: true,
            x_align: Clutter.ActorAlign.FILL,
            y_align: Clutter.ActorAlign.FILL,
            content_gravity: Clutter.ContentGravity.RESIZE_ASPECT
        });
        this.add_child(this._imageActor);

        this._controlsHUD = new GnomeLensVideoControls(this);
        this._controlsHUD.x_expand = true;
        this._controlsHUD.y_expand = true;
        this._controlsHUD.x_align = Clutter.ActorAlign.FILL;
        this._controlsHUD.y_align = Clutter.ActorAlign.END;
        this.add_child(this._controlsHUD);

        this.connectObject('captured-event', (actor, event) => {
            let type = event.type();
            if (type === Clutter.EventType.MOTION || type === Clutter.EventType.BUTTON_PRESS || type === Clutter.EventType.SCROLL) {
                this._resetHideTimer();
            }
            return Clutter.EVENT_PROPAGATE;
        }, this);

        this.connectObject('destroy', () => this._onDestroy(), this);
        this._resetHideTimer();
        
        ensureHistoryLoaded().then(() => {
            if (this._isDestroyed) return;
            this._currentTimeNs = PlaybackHistory.get(this._filepath) || 0;
            this._hasRestoredPosition = (this._currentTimeNs === 0);
            this._startGstVideo();
        });
    }

    _resetHideTimer() {
        if (this._controlsHUD.opacity === 0) {
            this._controlsHUD.remove_all_transitions();
            this._controlsHUD.ease({ opacity: 255, duration: 150, mode: Clutter.AnimationMode.EASE_OUT_QUAD });
        }
        
        if (this._hideTimerId > 0) {
            GLib.source_remove(this._hideTimerId);
        }
        
        this._hideTimerId = GLib.timeout_add(GLib.PRIORITY_DEFAULT, 1500, () => {
            this._hideTimerId = 0;
            this._controlsHUD.remove_all_transitions();
            this._controlsHUD.ease({ opacity: 0, duration: 250, mode: Clutter.AnimationMode.EASE_OUT_QUAD });
            return GLib.SOURCE_REMOVE;
        });
    }

    saveCurrentPosition() {
        if (this._pipeline && this._hasRestoredPosition) {
            let [success, pos] = this._pipeline.query_position(Gst.Format.TIME);
            if (success && pos > 0) {
                this._currentTimeNs = pos;
            }
        }
        if (this._currentTimeNs > 0) {
            PlaybackHistory.set(this._filepath, this._currentTimeNs);
            savePlaybackHistoryAsync();
        }
    }

    scrub(offset, isPercentage = false) {
        this._resetHideTimer();

        if (this._pipeline) {
            let pos;
            if (this._isSeeking && this._targetSeekNs !== undefined) {
                pos = this._targetSeekNs;
            } else {
                let [success, qpos] = this._pipeline.query_position(Gst.Format.TIME);
                pos = (success && this._hasRestoredPosition) ? qpos : this._currentTimeNs;
            }

            let targetNs;
            if (isPercentage) {
                if (this._durationNs <= 0) return;
                targetNs = Math.round(pos + (this._durationNs * offset));
            } else {
                targetNs = Math.round(pos + (offset * 1000000000));
            }

            if (targetNs < 0) targetNs = 0;
            if (this._durationNs > 0 && targetNs > this._durationNs) targetNs = this._durationNs;

            this._targetSeekNs = targetNs;
            this._isSeeking = true;
            this._pipeline.seek_simple(Gst.Format.TIME, Gst.SeekFlags.FLUSH, targetNs);
        } else {
            if (isPercentage) {
                if (this._durationNs > 0) {
                    this._currentTimeNs = Math.max(0, Math.round(this._currentTimeNs + (this._durationNs * offset)));
                    if (this._currentTimeNs > this._durationNs) this._currentTimeNs = this._durationNs;
                } else {
                    let fallbackOffset = offset > 0 ? 30 : -30;
                    this._currentTimeNs = Math.max(0, Math.round(this._currentTimeNs + (fallbackOffset * 1000000000)));
                }
            } else {
                this._currentTimeNs = Math.max(0, Math.round(this._currentTimeNs + (offset * 1000000000)));
            }
            this._extractFrameAndScheduleNext(true);
        }
    }

    seekToPercentage(percentage) {
        this._resetHideTimer();
        if (this._durationNs <= 0) return;

        let targetNs = Math.floor(this._durationNs * percentage);
        
        if (this._pipeline) {
            this._targetSeekNs = targetNs;
            this._isSeeking = true;
            this._pipeline.seek_simple(Gst.Format.TIME, Gst.SeekFlags.FLUSH, targetNs);
        } else {
            this._currentTimeNs = targetNs;
            this._extractFrameAndScheduleNext(true);
        }
    }

    setVolumeLevel(volume) {
        this._resetHideTimer();
        this._volumeLevel = Math.max(0.0, Math.min(1.0, volume));
        if (this._pipeline && !this._isMuted) {
            this._pipeline.set_property('volume', this._volumeLevel);
        }
        this._updateHUD();
    }

    toggleMute() {
        this._resetHideTimer();
        this._isMuted = !this._isMuted;
        if (this._pipeline) {
            this._pipeline.set_property('volume', this._isMuted ? 0.0 : this._volumeLevel);
        }
        this._updateHUD();
    }

    _updateHUD() {
        if (this._controlsHUD && typeof this._controlsHUD.updateUIState === 'function') {
            this._controlsHUD.updateUIState(this._currentTimeNs, this._durationNs, this._volumeLevel, this._isMuted);
        }
    }

    _onDestroy() {
        this._isDestroyed = true;
        this.saveCurrentPosition();
        this._stopVideo();
    }

    _stopVideo() {
        if (this._hideTimerId > 0) {
            GLib.source_remove(this._hideTimerId);
            this._hideTimerId = 0;
        }
        if (this._playbackTimerId > 0) {
            GLib.source_remove(this._playbackTimerId);
            this._playbackTimerId = 0;
        }
        if (this._idleRenderId > 0) {
            GLib.source_remove(this._idleRenderId);
            this._idleRenderId = 0;
        }

        if (this._pipeline) {
            let bus = this._pipeline.get_bus();
            if (this._busWatchId > 0 && bus) {
                bus.disconnect(this._busWatchId);
                bus.remove_signal_watch();
                this._busWatchId = 0;
            }
            this._pipeline.set_state(Gst.State.NULL);
            this._pipeline = null;
            this._sink = null;
        }

        if (this._proc) {
            this._proc.force_exit();
            this._proc = null;
        }

        if (this._lastTempFile) {
            let file = Gio.File.new_for_path(this._lastTempFile);
            file.delete_async(GLib.PRIORITY_DEFAULT, null, (f, res) => {
                try { f.delete_finish(res); } catch(e) { console.debug(`[Lens for GNOME] temp file delete failed: ${e.message}`); }
            });
            this._lastTempFile = null;
        }
    }

    async _startGstVideo() {
        let hasGst = await ensureGst();
        if (!hasGst) {
            this._extractFrameAndScheduleNext();
            return;
        }

        this._stopVideo();

        try {
            let pipeline = Gst.ElementFactory.make('playbin', null);
            if (!pipeline) throw new Error("Could not construct playbin element layer instance");

            pipeline.set_property('flags', 1 | 2); 
            pipeline.set_property('uri', Gio.File.new_for_path(this._filepath).get_uri());
            pipeline.set_property('volume', this._isMuted ? 0.0 : this._volumeLevel);

            let sink = Gst.ElementFactory.make('appsink', null);
            if (!sink) throw new Error("Could not construct appsink element container");

            let caps = Gst.Caps.from_string('video/x-raw, format=RGBA');
            sink.set_property('caps', caps);
            sink.set_property('drop', true);
            sink.set_property('max-buffers', 1);
            sink.set_property('emit-signals', false);

            pipeline.set_property('video-sink', sink);
            
            this._pipeline = pipeline;
            this._sink = sink;

            let bus = pipeline.get_bus();
            bus.add_signal_watch();
            this._busWatchId = bus.connect('message', (busMsg, message) => {
                if (message.type === Gst.MessageType.DURATION_CHANGED) {
                    let [success, dur] = this._pipeline.query_duration(Gst.Format.TIME);
                    if (success) this._durationNs = dur;
                } else if (message.type === Gst.MessageType.ASYNC_DONE) {
                    this._isSeeking = false;
                    if (!this._hasRestoredPosition) {
                        this._pipeline.seek_simple(Gst.Format.TIME, Gst.SeekFlags.FLUSH, this._currentTimeNs);
                        this._hasRestoredPosition = true;
                    }
                } else if (message.type === Gst.MessageType.EOS) {
                    if (this._pipeline) {
                        this._currentTimeNs = 0;
                        PlaybackHistory.delete(this._filepath);
                        savePlaybackHistoryAsync();
                        this._pipeline.seek_simple(Gst.Format.TIME, Gst.SeekFlags.FLUSH, 0);
                    }
                } else if (message.type === Gst.MessageType.ERROR) {
                    this._isSeeking = false;
                    if (this._pipeline) this._extractFrameAndScheduleNext();
                }
            });

            let stateReturn = pipeline.set_state(Gst.State.PLAYING);
            if (stateReturn === Gst.StateChangeReturn.FAILURE) {
                throw new Error("Pipeline set_state failure");
            }

            this._playbackTimerId = GLib.timeout_add(GLib.PRIORITY_DEFAULT, 16, () => {
                if (!this._sink || !this.visible || !this._pipeline) return GLib.SOURCE_CONTINUE;
                
                if (this._hasRestoredPosition) {
                    let [successPos, pos] = this._pipeline.query_position(Gst.Format.TIME);
                    if (successPos) this._currentTimeNs = pos;
                }

                let [successDur, dur] = this._pipeline.query_duration(Gst.Format.TIME);
                if (successDur) this._durationNs = dur;

                let sample = null;
                try {
                    if (typeof this._sink.try_pull_sample === 'function') {
                        sample = this._sink.try_pull_sample(0);
                    } else {
                        sample = this._sink.emit('try-pull-sample', 0);
                    }
                } catch (e) { console.debug(`[Lens for GNOME] Frame sample extraction blocked: ${e.message}`); }
                
                if (sample) {
                    if (this._idleRenderId === 0) {
                        this._idleRenderId = GLib.idle_add(GLib.PRIORITY_DEFAULT_IDLE, () => {
                            this._idleRenderId = 0;
                            if (sample && this.visible && this._pipeline) {
                                this._renderSample(sample);
                            }
                            return GLib.SOURCE_REMOVE;
                        });
                    }
                }
                
                this._updateHUD();
                return GLib.SOURCE_CONTINUE;
            });

        } catch (e) {
            this._extractFrameAndScheduleNext();
        }
    }

    _renderSample(sample) {
        if (!this.visible || !this._pipeline || !sample) return;

        let caps = sample.get_caps();
        if (!caps) return;
        
        let structure = caps.get_structure(0);
        if (!structure) return;

        let width = 0, height = 0;
        let [successW, w] = structure.get_int('width');
        let [successH, h] = structure.get_int('height');
        
        if (successW && successH) {
            width = w; height = h;
        }
        
        if (!width || !height || width <= 0 || height <= 0) return;

        let buffer = sample.get_buffer();
        if (!buffer) return;

        let [isMapped, mapInfo] = buffer.map(Gst.MapFlags.READ);
        if (isMapped) {
            let data = mapInfo.data;

            if (!this._imageContent || this._contentWidth !== width || this._contentHeight !== height) {
                if (typeof St.ImageContent.new_with_preferred_size === 'function') {
                    this._imageContent = St.ImageContent.new_with_preferred_size(width, height);
                } else {
                    this._imageContent = new St.ImageContent();
                }
                this._contentWidth = width;
                this._contentHeight = height;
                this._imageActor.set_content(this._imageContent);
            }
            
            let pixelFormat = Cogl.PixelFormat.RGBA_8888;
            let bytesSuccess = false;
            let glibBytes = (data instanceof GLib.Bytes) ? data : new GLib.Bytes(data);
            let coglCtx = null;

            if (global.stage && global.stage.context) {
                let backend = global.stage.context.get_backend();
                if (backend && typeof backend.get_cogl_context === 'function') {
                    coglCtx = backend.get_cogl_context();
                }
            }

            if (coglCtx) {
                try {
                    bytesSuccess = this._imageContent.set_bytes(coglCtx, glibBytes, pixelFormat, width, height, width * 4);
                } catch(e1) {
                    bytesSuccess = this._imageContent.set_bytes(glibBytes, pixelFormat, width, height, width * 4);
                }
            } else {
                bytesSuccess = this._imageContent.set_bytes(glibBytes, pixelFormat, width, height, width * 4);
            }
            
            if (bytesSuccess) {
                this._imageActor.queue_redraw();
            }

            buffer.unmap(mapInfo);
        }
    }

    _extractFrameAndScheduleNext(isScrubbing = false) {
        if (this._playbackTimerId > 0) {
            GLib.source_remove(this._playbackTimerId);
            this._playbackTimerId = 0;
        }

        if (this._proc) {
            this._proc.force_exit();
            this._proc = null;
        }
        
        let secondsCounter = Math.floor(this._currentTimeNs / 1000000000);
        let tempFile = GLib.build_filenamev([GLib.get_tmp_dir(), `lens-for-gnome-preview-${GLib.uuid_string_random()}.jpg`]);

        let cmd = ['ffmpeg', '-y', '-ss', secondsCounter.toString(), '-i', this._filepath, '-vframes', '1', '-q:v', '2', '-vf', 'scale=640:-1', tempFile];

        try {
            let proc = Gio.Subprocess.new(cmd, Gio.SubprocessFlags.STDOUT_SILENCE | Gio.SubprocessFlags.STDERR_SILENCE);
            this._proc = proc;

            proc.wait_async(null, (p, res) => {
                try { 
                    proc.wait_finish(res); 
                } catch(e) { console.debug(`[Lens for GNOME] ffmpeg extraction subprocess terminated: ${e.message}`); }
                
                if (this._proc !== proc) return;
                this._proc = null;

                let file = Gio.File.new_for_path(tempFile);
                file.query_info_async(Gio.FILE_ATTRIBUTE_STANDARD_TYPE, Gio.FileQueryInfoFlags.NONE, GLib.PRIORITY_DEFAULT, null, (f, resInfo) => {
                    let exists = false;
                    try {
                        f.query_info_finish(resInfo);
                        exists = true;
                    } catch (e) { console.debug(`[Lens for GNOME] ffmpeg query info failed: ${e.message}`); }

                    if (exists) {
                        if (!this.visible) {
                            f.delete_async(GLib.PRIORITY_DEFAULT, null, (df, dres) => { try { df.delete_finish(dres); } catch(e) { console.debug(`[Lens for GNOME] Unused file cleanup failed: ${e.message}`); } });
                            return;
                        }
                        
                        this._imageActor.set_content(null);
                        this._imageActor.style = `background-image: url("file://${tempFile}"); background-size: contain; background-repeat: no-repeat;`;
                        
                        if (this._lastTempFile) {
                            let lastFile = Gio.File.new_for_path(this._lastTempFile);
                            lastFile.delete_async(GLib.PRIORITY_DEFAULT, null, (df, dres) => { try { df.delete_finish(dres); } catch(e) { console.debug(`[Lens for GNOME] Previous temp file deletion failed: ${e.message}`); } });
                        }
                        this._lastTempFile = tempFile;
                    }

                    this._updateHUD();

                    this._playbackTimerId = GLib.timeout_add(GLib.PRIORITY_DEFAULT, 1000, () => {
                        this._playbackTimerId = 0;
                        this._currentTimeNs += 1000000000;
                        this._extractFrameAndScheduleNext();
                        return GLib.SOURCE_REMOVE;
                    });
                });
            });

        } catch (e) {
            this._proc = null;
        }
    }
});