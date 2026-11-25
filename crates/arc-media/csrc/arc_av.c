/*
 * arc_av — Arcline's narrow libav shim (docs/04 §1, ADR-0003).
 *
 * Rust never touches FFmpeg structs: it holds an opaque `ArcAv*` and calls these functions.
 * All struct access happens here, compiled against the real FFmpeg headers, so FFmpeg ABI
 * details cannot leak into Rust.
 *
 * Video: frame-accurate decode to packed RGBA (sRGB, BT.709/601 per stream tags).
 * Audio: decode + resample to interleaved f32 stereo at a requested rate.
 *
 * Return codes: 0 = ok, 1 = end of stream, < 0 = error (message in `err`).
 */
#include <libavformat/avformat.h>
#include <libavcodec/avcodec.h>
#include <libavutil/avutil.h>
#include <libavutil/imgutils.h>
#include <libavutil/opt.h>
#include <libswscale/swscale.h>
#include <libswresample/swresample.h>
#include <stdint.h>
#include <string.h>
#include <stdio.h>

/* Compile-time version pin (spec docs/specs/ffmpeg-9.1-upgrade.md §4.3). The shim is written for
 * FFmpeg 9.x only; build.rs gives the friendlier message, this is the backstop. */
#if LIBAVUTIL_VERSION_MAJOR != 61 || LIBAVCODEC_VERSION_MAJOR != 63 || \
    LIBAVFORMAT_VERSION_MAJOR != 63 || LIBSWSCALE_VERSION_MAJOR != 10 || \
    LIBSWRESAMPLE_VERSION_MAJOR != 7
#error "arc_av requires FFmpeg 9.x headers (lavu 61, lavc 63, lavf 63, sws 10, swr 7); see docs/specs/ffmpeg-9.1-upgrade.md"
#endif
#if LIBAVCODEC_VERSION_INT < AV_VERSION_INT(63, 24, 100) || LIBAVFORMAT_VERSION_INT < AV_VERSION_INT(63, 16, 100)
#error "arc_av requires FFmpeg 9.1 or newer within the 9.x series"
#endif

typedef struct ArcAv {
    AVFormatContext *fmt;
    AVCodecContext *dec;
    AVPacket *pkt;
    AVFrame *frame;
    AVFrame *cur;         /* last delivered frame */
    AVFrame *held;        /* look-ahead: first decoded frame after `cur` */
    int drained;          /* decoder returned EOF */
    int pkt_pending;      /* the packet in `pkt` must be resent before reading another */
    int stream;
    int is_audio;
    struct SwsContext *sws;
    int sws_w, sws_h, sws_fmt;
    int dst_w, dst_h;     /* stream dimensions reported in ArcAvInfo: the RGBA destination size */
    struct SwrContext *swr;
    int out_rate;
    int swr_in_fmt, swr_in_rate;  /* input parameters of the current resampler */
    AVChannelLayout swr_in_layout;
    int64_t last_pts;     /* pts of the last delivered video frame, INT64_MIN if none */
    int eof_sent;
    /* buffered resampled audio (interleaved stereo f32): abuf[0] is output sample `abuf_start` */
    float *abuf;
    int abuf_len, abuf_cap;
    int64_t abuf_start;
    int audio_started;    /* a decode position has been established */
    int anchor_pending;   /* the next appended frame fixes abuf_start from its timestamp */
} ArcAv;

typedef struct ArcAvInfo {
    int width, height;
    int fr_num, fr_den;      /* nominal frame rate (avg_frame_rate, else r_frame_rate) */
    int tb_num, tb_den;      /* stream time base */
    int64_t start_pts;       /* stream start time in tb (0 if unknown) */
    int64_t duration_us;     /* best-known duration in microseconds (-1 unknown) */
    int sample_rate, channels;
    int vfr;                 /* 1 if avg and r frame rates disagree */
    char codec[32];
    char pix_fmt[32];
} ArcAvInfo;

/* Run-time identity of the loaded libraries (spec §4.5). Signatures are part of the FFI. */
typedef struct ArcAvRuntime {
    unsigned build[5];    /* LIB*_VERSION_INT at compile time: avutil, avcodec, avformat, swscale, swresample */
    unsigned runtime[5];  /* avutil_version(), avcodec_version(), ... at run time */
    char license[5][48];  /* av*_license() */
    char ident[64];       /* av_version_info(), e.g. "n9.1" */
} ArcAvRuntime;

int arc_av_abi_size(void) { return (int)sizeof(ArcAvInfo); }

static void copy_str(char *dst, size_t len, const char *src) {
    snprintf(dst, len, "%s", src ? src : "");
}

int arc_av_runtime(ArcAvRuntime *out) {
    if (!out) return -1;
    memset(out, 0, sizeof *out);
    out->build[0] = LIBAVUTIL_VERSION_INT;
    out->build[1] = LIBAVCODEC_VERSION_INT;
    out->build[2] = LIBAVFORMAT_VERSION_INT;
    out->build[3] = LIBSWSCALE_VERSION_INT;
    out->build[4] = LIBSWRESAMPLE_VERSION_INT;
    out->runtime[0] = avutil_version();
    out->runtime[1] = avcodec_version();
    out->runtime[2] = avformat_version();
    out->runtime[3] = swscale_version();
    out->runtime[4] = swresample_version();
    copy_str(out->license[0], sizeof out->license[0], avutil_license());
    copy_str(out->license[1], sizeof out->license[0], avcodec_license());
    copy_str(out->license[2], sizeof out->license[0], avformat_license());
    copy_str(out->license[3], sizeof out->license[0], swscale_license());
    copy_str(out->license[4], sizeof out->license[0], swresample_license());
    copy_str(out->ident, sizeof out->ident, av_version_info());
    return 0;
}

static void set_err(char *err, int len, const char *what, int code) {
    if (!err || len <= 0) return;
    char buf[256] = {0};
    if (code) av_strerror(code, buf, sizeof buf);
    snprintf(err, (size_t)len, "%s%s%s", what, code ? ": " : "", buf);
}

void arc_av_close(ArcAv *a) {
    if (!a) return;
    if (a->sws) sws_freeContext(a->sws);
    if (a->swr) swr_free(&a->swr);
    av_channel_layout_uninit(&a->swr_in_layout);
    if (a->held) av_frame_free(&a->held);
    if (a->cur) av_frame_free(&a->cur);
    if (a->frame) av_frame_free(&a->frame);
    if (a->pkt) av_packet_free(&a->pkt);
    if (a->dec) avcodec_free_context(&a->dec);
    if (a->fmt) avformat_close_input(&a->fmt);
    av_free(a->abuf);
    av_free(a);
}

/* kind: 0 = video, 1 = audio. threads: decoder threads (0 = auto). */
int arc_av_open(const char *path, int kind, int threads, ArcAv **out, char *err, int errlen) {
    *out = NULL;
    av_log_set_level(AV_LOG_ERROR);
    /* Only local files: the caller builds `file:` URLs from canonical paths, and the whitelist is
     * inherited by nested opens (playlists, references). Rejects http:, handle:, concat:, ... even
     * if a binary contains those protocols (audit F-09, spec S1). */
    if (!path || strncmp(path, "file:", 5) != 0) {
        set_err(err, errlen, "open: only local file: paths are allowed", 0);
        return -1;
    }
    ArcAv *a = av_mallocz(sizeof *a);
    if (!a) { set_err(err, errlen, "out of memory", 0); return -1; }
    a->last_pts = INT64_MIN;
    a->is_audio = kind == 1;
    AVDictionary *opts = NULL;
    if (av_dict_set(&opts, "protocol_whitelist", "file", 0) < 0) {
        set_err(err, errlen, "open: out of memory", 0);
        av_dict_free(&opts);
        av_free(a);
        return -1;
    }
    int r = avformat_open_input(&a->fmt, path, NULL, &opts);
    av_dict_free(&opts);
    if (r < 0) { set_err(err, errlen, "open", r); arc_av_close(a); return -1; }
    r = avformat_find_stream_info(a->fmt, NULL);
    if (r < 0) { set_err(err, errlen, "find_stream_info", r); arc_av_close(a); return -1; }
    const AVCodec *codec = NULL;
    r = av_find_best_stream(a->fmt, a->is_audio ? AVMEDIA_TYPE_AUDIO : AVMEDIA_TYPE_VIDEO, -1, -1, &codec, 0);
    if (r < 0) { set_err(err, errlen, a->is_audio ? "no audio stream" : "no video stream", r); arc_av_close(a); return -1; }
    a->stream = r;
    for (unsigned i = 0; i < a->fmt->nb_streams; i++)
        if ((int)i != a->stream) a->fmt->streams[i]->discard = AVDISCARD_ALL;
    a->dec = avcodec_alloc_context3(codec);
    if (!a->dec) { set_err(err, errlen, "alloc codec context", 0); arc_av_close(a); return -1; }
    r = avcodec_parameters_to_context(a->dec, a->fmt->streams[a->stream]->codecpar);
    if (r < 0) { set_err(err, errlen, "codec parameters", r); arc_av_close(a); return -1; }
    /* The buffer Rust allocates is the size reported by arc_av_info (the stream's codec
     * parameters), so every decoded frame is scaled to exactly this size. Frames can change size
     * mid-stream (audit F-02). */
    a->dst_w = a->fmt->streams[a->stream]->codecpar->width;
    a->dst_h = a->fmt->streams[a->stream]->codecpar->height;
    a->dec->thread_count = threads;
    a->dec->pkt_timebase = a->fmt->streams[a->stream]->time_base;
    r = avcodec_open2(a->dec, codec, NULL);
    if (r < 0) { set_err(err, errlen, "open decoder", r); arc_av_close(a); return -1; }
    a->pkt = av_packet_alloc();
    a->frame = av_frame_alloc();
    a->held = av_frame_alloc();
    a->cur = av_frame_alloc();
    if (!a->pkt || !a->frame || !a->held || !a->cur) { set_err(err, errlen, "alloc frame", 0); arc_av_close(a); return -1; }
    *out = a;
    return 0;
}

int arc_av_info(ArcAv *a, ArcAvInfo *info) {
    memset(info, 0, sizeof *info);
    AVStream *s = a->fmt->streams[a->stream];
    AVCodecParameters *p = s->codecpar;
    info->width = p->width;
    info->height = p->height;
    AVRational fr = s->avg_frame_rate.num ? s->avg_frame_rate : s->r_frame_rate;
    info->fr_num = fr.num;
    info->fr_den = fr.den;
    info->vfr = s->avg_frame_rate.num && s->r_frame_rate.num && av_cmp_q(s->avg_frame_rate, s->r_frame_rate) != 0;
    info->tb_num = s->time_base.num;
    info->tb_den = s->time_base.den;
    info->start_pts = s->start_time == AV_NOPTS_VALUE ? 0 : s->start_time;
    if (s->duration != AV_NOPTS_VALUE)
        info->duration_us = av_rescale_q(s->duration, s->time_base, AV_TIME_BASE_Q);
    else if (a->fmt->duration != AV_NOPTS_VALUE)
        info->duration_us = a->fmt->duration;
    else
        info->duration_us = -1;
    info->sample_rate = p->sample_rate;
    info->channels = p->ch_layout.nb_channels;
    snprintf(info->codec, sizeof info->codec, "%s", avcodec_get_name(p->codec_id));
    const char *pf = p->format >= 0 ? (a->is_audio ? av_get_sample_fmt_name(p->format) : av_get_pix_fmt_name(p->format)) : NULL;
    snprintf(info->pix_fmt, sizeof info->pix_fmt, "%s", pf ? pf : "");
    return 0;
}

/* Pull the next decoded frame into a->frame. 0 ok, 1 eof, <0 error. */
static int next_frame(ArcAv *a, char *err, int errlen) {
    for (;;) {
        int r = avcodec_receive_frame(a->dec, a->frame);
        if (r == 0) return 0;
        if (r == AVERROR_EOF) { a->drained = 1; return 1; }
        if (r != AVERROR(EAGAIN)) { set_err(err, errlen, "receive_frame", r); return -1; }
        if (a->eof_sent) return 1;
        if (!a->pkt_pending) {
            r = av_read_frame(a->fmt, a->pkt);
            if (r == AVERROR_EOF) {
                avcodec_send_packet(a->dec, NULL);
                a->eof_sent = 1;
                continue;
            }
            if (r < 0) { set_err(err, errlen, "read_frame", r); return -1; }
            if (a->pkt->stream_index != a->stream) {
                av_packet_unref(a->pkt);
                continue;
            }
        }
        r = avcodec_send_packet(a->dec, a->pkt);
        if (r == AVERROR(EAGAIN)) {
            /* The decoder wants a drained frame first: keep the packet and resend it on the next
             * round. It used to be unreferenced and lost (audit F-28). */
            a->pkt_pending = 1;
            r = avcodec_receive_frame(a->dec, a->frame);
            if (r == 0) return 0;
            if (r == AVERROR_EOF) { a->drained = 1; return 1; }
            if (r != AVERROR(EAGAIN)) { set_err(err, errlen, "receive_frame", r); return -1; }
            continue;
        }
        av_packet_unref(a->pkt);
        a->pkt_pending = 0;
        if (r < 0 && r != AVERROR_INVALIDDATA) {
            set_err(err, errlen, "send_packet", r);
            return -1;
        }
    }
}

static int64_t frame_pts(const AVFrame *f) {
    return f->best_effort_timestamp != AV_NOPTS_VALUE ? f->best_effort_timestamp : f->pts;
}

static int to_rgba(ArcAv *a, const AVFrame *f, uint8_t *dst, int stride) {
    if (a->dst_w <= 0 || a->dst_h <= 0) return -1;
    /* `dst` holds dst_h rows of dst_w RGBA pixels; a smaller stride would overflow every row. */
    if (stride < a->dst_w * 4) return -1;
    if (!a->sws || a->sws_w != f->width || a->sws_h != f->height || a->sws_fmt != f->format) {
        if (a->sws) sws_freeContext(a->sws);
        a->sws = sws_getContext(f->width, f->height, f->format, a->dst_w, a->dst_h, AV_PIX_FMT_RGBA,
                                SWS_BICUBIC | SWS_ACCURATE_RND | SWS_FULL_CHR_H_INT, NULL, NULL, NULL);
        if (!a->sws) return -1;
        a->sws_w = f->width;
        a->sws_h = f->height;
        a->sws_fmt = f->format;
        int cs = f->colorspace;
        if (cs == AVCOL_SPC_UNSPECIFIED) cs = f->height >= 720 ? AVCOL_SPC_BT709 : AVCOL_SPC_SMPTE170M;
        int src_range = f->color_range == AVCOL_RANGE_JPEG ? 1 : 0;
        sws_setColorspaceDetails(a->sws, sws_getCoefficients(cs), src_range, sws_getCoefficients(SWS_CS_ITU709), 1, 0, 1 << 16, 1 << 16);
    }
    uint8_t *planes[4] = {dst, NULL, NULL, NULL};
    int strides[4] = {stride, 0, 0, 0};
    /* Scale to the reported size even when the frame itself changed size or format. */
    return sws_scale(a->sws, (const uint8_t *const *)f->data, f->linesize, 0, f->height, planes, strides) == a->dst_h ? 0 : -1;
}

/* Stream start in stream ticks; the earliest position a seek can usefully ask for. */
static int64_t stream_start_ticks(const ArcAv *a) {
    const AVStream *s = a->fmt->streams[a->stream];
    return s->start_time == AV_NOPTS_VALUE ? 0 : s->start_time;
}

/* Seek to `target` and drop the decoder and look-ahead state. */
static int video_seek(ArcAv *a, int64_t target, char *err, int errlen) {
    /* A target at or before the stream start must rewind to the first packet: a timestamp seek in
     * an unindexed container (MPEG-TS) can land after the target even for position 0. The
     * global (stream_index -1) form takes AV_TIME_BASE and is the reliable "rewind" (F-08). */
    int r = target <= stream_start_ticks(a)
                ? av_seek_frame(a->fmt, -1, 0, AVSEEK_FLAG_BACKWARD)
                : av_seek_frame(a->fmt, a->stream, target, AVSEEK_FLAG_BACKWARD);
    if (r < 0) { set_err(err, errlen, "seek", r); return -1; }
    avcodec_flush_buffers(a->dec);
    a->eof_sent = 0;
    a->drained = 0;
    av_frame_unref(a->cur);
    av_frame_unref(a->held);
    return 0;
}

/*
 * Frame-accurate video decode: deliver the frame whose interval contains `target_pts`
 * (the last frame with pts <= target). Sequential and repeated requests are served without
 * seeking; backward or far-forward requests seek to the preceding keyframe and decode forward.
 * `dst` must hold width*height*4 bytes. `got_pts` receives the delivered frame's pts.
 *
 * A single seek is not enough on every container (audit F-08): the MP4 index is keyed by DTS, so
 * a seek for a frame just before a keyframe can land past it, and MPEG-TS has no index at all.
 * When the first decoded frame is later than the target, seek again further back and decode
 * forward; only a target at or before the stream start may resolve to the stream's first frame.
 */
int arc_av_video_at(ArcAv *a, int64_t target_pts, uint8_t *dst, int stride, int64_t *got_pts, char *err, int errlen) {
    if (a->is_audio) { set_err(err, errlen, "not a video decoder", 0); return -1; }
    AVStream *s = a->fmt->streams[a->stream];
    const int64_t start = stream_start_ticks(a);
    int64_t ahead_limit = av_rescale_q(2 * AV_TIME_BASE, AV_TIME_BASE_Q, s->time_base);
    int64_t seek_to = target_pts;
    int retry = 0;
    for (;;) {
        int have_cur = a->cur->buf[0] != NULL;
        int have_held = a->held->buf[0] != NULL;
        int64_t cur_pts = have_cur ? frame_pts(a->cur) : INT64_MIN;
        int64_t held_pts = have_held ? frame_pts(a->held) : INT64_MAX;

        /* 1. the current frame still covers the target */
        if (!retry && have_cur && cur_pts <= target_pts && (have_held ? target_pts < held_pts : a->drained)) {
            if (to_rgba(a, a->cur, dst, stride) < 0) { set_err(err, errlen, "convert", 0); return -1; }
            *got_pts = cur_pts;
            a->last_pts = cur_pts;
            return 0;
        }
        /* 2. seek if going backwards, far ahead, or retrying an overshooting seek */
        int64_t pos = have_cur ? cur_pts : (have_held ? held_pts : INT64_MIN);
        int need_seek = retry || (!have_cur && !have_held) || target_pts < pos ||
                        target_pts - pos > ahead_limit;
        if (need_seek) {
            if (video_seek(a, seek_to, err, errlen) < 0) return -1;
            have_cur = have_held = 0;
        }
        /* 3. advance: the look-ahead frame becomes current if it is not past the target */
        if (have_held && frame_pts(a->held) <= target_pts) {
            av_frame_unref(a->cur);
            av_frame_move_ref(a->cur, a->held);
            have_cur = 1;
            have_held = 0;
        }
        while (!have_held) {
            int r = next_frame(a, err, errlen);
            if (r < 0) return r;
            if (r == 1) break;
            int64_t pts = frame_pts(a->frame);
            if (pts == AV_NOPTS_VALUE) { av_frame_unref(a->frame); continue; }
            if (pts <= target_pts) {
                av_frame_unref(a->cur);
                av_frame_move_ref(a->cur, a->frame);
                have_cur = 1;
            } else {
                av_frame_move_ref(a->held, a->frame);
                have_held = 1;
            }
        }
        if (have_cur) break;
        /* A seek can land past the last decodable frame (unindexed MPEG-TS): back off and retry
         * instead of reporting end of stream. Only a target at the stream start may resolve to the
         * first frame when the first decoded frame is already later than it. */
        if (seek_to <= start) {
            if (!have_held) return 1;           /* no frames from the start: end of stream */
            av_frame_move_ref(a->cur, a->held);
            have_held = 0;
            break;
        }
        /* The seek landed after the target: back off and decode across the target frame. */
        retry++;
        int64_t backoff = av_rescale_q((int64_t)500000 << (retry > 6 ? 6 : retry - 1),
                                       AV_TIME_BASE_Q, s->time_base);
        seek_to = target_pts > start + backoff ? target_pts - backoff : start;
        if (retry > 12) { set_err(err, errlen, "seek: no frame at or before the target", 0); return -1; }
    }
    if (to_rgba(a, a->cur, dst, stride) < 0) { set_err(err, errlen, "convert", 0); return -1; }
    *got_pts = frame_pts(a->cur);
    a->last_pts = *got_pts;
    return 0;
}

/* ---- audio ---------------------------------------------------------------------------------- */

/* Output frames kept behind the read cursor. The mixer reads one frame more than it advances, so
 * consecutive blocks overlap by one sample; serving that overlap from the buffer instead of
 * re-seeking removes the per-block decoder reset (audit F-06). One second of history also covers
 * small backward jumps. */
#define ARC_AUDIO_HISTORY 48000
/* Frames decoded and discarded before a seek target so that MDCT decoders (AAC, MP3, Opus) have
 * re-primed and the resampler has warmed up before the first delivered sample (audit F-06). */
#define ARC_AUDIO_PREROLL 12000
static int abuf_push(ArcAv *a, int frames) {
    int need = (a->abuf_len + frames) * 2;
    if (need > a->abuf_cap) {
        int cap = need * 2;
        float *n = av_realloc(a->abuf, (size_t)cap * sizeof(float));
        if (!n) return -1;
        a->abuf = n;
        a->abuf_cap = cap;
    }
    return 0;
}

/* Flush the samples still buffered inside the current resampler into `abuf`, so a rebuild on an
 * input parameter change does not create a gap or repeat samples (F-03). */
static int swr_drain(ArcAv *a) {
    if (!a->swr) return 0;
    int maxo = swr_get_out_samples(a->swr, 0);
    if (maxo <= 0) return 0;
    if (abuf_push(a, maxo) < 0) return AVERROR(ENOMEM);
    uint8_t *outp = (uint8_t *)(a->abuf + (size_t)a->abuf_len * 2);
    int got = swr_convert(a->swr, &outp, maxo, NULL, 0);
    if (got < 0) return got;
    a->abuf_len += got;
    return 0;
}

/*
 * Configure the resampler for this frame's input parameters. Any change of format, sample rate or
 * channel layout rebuilds it, and the old one is drained first: frames can change parameters
 * mid-stream and feeding a mono frame to a stereo resampler is invalid (audit F-03).
 */
static int ensure_swr(ArcAv *a, const AVFrame *f, int out_rate) {
    /* av_channel_layout_copy() frees the destination first: it must be zero-initialised. */
    AVChannelLayout in = {0};
    if (av_channel_layout_copy(&in, &f->ch_layout) < 0) return AVERROR(ENOMEM);
    if (in.nb_channels == 0) av_channel_layout_default(&in, 2);
    if (a->swr && a->out_rate == out_rate && a->swr_in_fmt == f->format &&
        a->swr_in_rate == f->sample_rate && av_channel_layout_compare(&a->swr_in_layout, &in) == 0) {
        av_channel_layout_uninit(&in);
        return 0;
    }
    if (a->swr) {
        int d = swr_drain(a);
        if (d < 0) { av_channel_layout_uninit(&in); return d; }
        swr_free(&a->swr);
    }
    av_channel_layout_uninit(&a->swr_in_layout);
    AVChannelLayout stereo = AV_CHANNEL_LAYOUT_STEREO;
    int r = swr_alloc_set_opts2(&a->swr, &stereo, AV_SAMPLE_FMT_FLT, out_rate, &in, f->format, f->sample_rate, 0, NULL);
    if (r >= 0) {
        /* high quality, deterministic resampler */
        av_opt_set_int(a->swr, "filter_size", 32, 0);
        av_opt_set_int(a->swr, "phase_shift", 10, 0);
        r = swr_init(a->swr);
    }
    if (r >= 0) r = av_channel_layout_copy(&a->swr_in_layout, &in);
    av_channel_layout_uninit(&in);
    if (r < 0) { swr_free(&a->swr); return r; }
    a->swr_in_fmt = f->format;
    a->swr_in_rate = f->sample_rate;
    a->out_rate = out_rate;
    return 0;
}

/* Absolute output-sample position of the first frame decoded after a reposition. Returns -1 when
 * the frame carries no timestamp. */
static int64_t audio_anchor(const ArcAv *a, const AVFrame *f, int out_rate) {
    const AVStream *s = a->fmt->streams[a->stream];
    int64_t pts = frame_pts(f);
    if (pts == AV_NOPTS_VALUE) return -1;
    int64_t base = s->start_time == AV_NOPTS_VALUE ? 0 : s->start_time;
    int64_t pos = av_rescale_q(pts - base, s->time_base, (AVRational){1, out_rate});
    return pos > 0 ? pos : 0;
}
/* Start a new decode position: seek to `target` minus the pre-roll and drop the buffered output.
 * The resampler is kept (it stays primed); its stale history is flushed by the pre-roll that is
 * decoded and discarded before `target` (audit F-06). */
static int audio_reposition(ArcAv *a, int out_rate, int64_t target, char *err, int errlen) {
    AVStream *s = a->fmt->streams[a->stream];
    int64_t pre = target > ARC_AUDIO_PREROLL ? target - ARC_AUDIO_PREROLL : 0;
    int r = 0;
    /* The very first read from the start needs no seek: the demuxer is already there and an
     * unnecessary seek costs the decoder its priming (audit F-06). */
    if (a->audio_started || target > 0) {
        int64_t ts = av_rescale_q(pre, (AVRational){1, out_rate}, s->time_base);
        if (s->start_time != AV_NOPTS_VALUE) ts += s->start_time;
        r = av_seek_frame(a->fmt, a->stream, ts, AVSEEK_FLAG_BACKWARD);
    }
    if (r < 0 && pre > 0) { set_err(err, errlen, "audio seek", r); return -1; }
    avcodec_flush_buffers(a->dec);
    a->eof_sent = 0;
    a->drained = 0;
    a->abuf_len = 0;
    a->abuf_start = target;
    a->audio_started = 1;
    a->anchor_pending = 1;
    return 0;
}

/* Decode one more frame and append its resampled output. 0 = appended, 1 = end of stream. */
static int audio_refill(ArcAv *a, int out_rate, char *err, int errlen) {
    int r = next_frame(a, err, errlen);
    if (r < 0) return r;
    if (r == 1) return 1;
    if (a->anchor_pending) {
        int64_t pos = audio_anchor(a, a->frame, out_rate);

        if (pos >= 0) {
            a->abuf_start = pos;
            a->abuf_len = 0;
            a->anchor_pending = 0;
        }
    }
    /* May drain the previous resampler into abuf before rebuilding it. */
    if (ensure_swr(a, a->frame, out_rate) < 0) {
        av_frame_unref(a->frame);
        set_err(err, errlen, "resampler", 0);
        return -1;
    }
    int maxo = swr_get_out_samples(a->swr, a->frame->nb_samples);
    if (abuf_push(a, maxo) < 0) { av_frame_unref(a->frame); set_err(err, errlen, "out of memory", 0); return -1; }
    uint8_t *outp = (uint8_t *)(a->abuf + (size_t)a->abuf_len * 2);
    int got = swr_convert(a->swr, &outp, maxo, (const uint8_t **)a->frame->extended_data, a->frame->nb_samples);
    av_frame_unref(a->frame);
    if (got < 0) { set_err(err, errlen, "resample", got); return -1; }
    a->abuf_len += got;
    return 0;
}

/*
 * Read `frames` stereo f32 frames starting at output sample `start` (at `out_rate`), counted from
 * the stream's first sample. Missing samples (before start / after end) are zero.
 *
 * Reads are served from a buffer with a history window, so overlapping and repeated reads do not
 * touch the decoder; only a genuine jump re-seeks, and that path decodes a pre-roll first
 * (audit F-06). Returns the number of frames that came from the stream (rest zero-filled).
 */
int arc_av_audio_read(ArcAv *a, int out_rate, int64_t start, int frames, float *dst, char *err, int errlen) {
    if (!a->is_audio) { set_err(err, errlen, "not an audio decoder", 0); return -1; }
    if (frames <= 0) return 0;
    if (start < 0) start = 0;
    memset(dst, 0, (size_t)frames * 2 * sizeof(float));
    int64_t end = a->abuf_start + a->abuf_len;
    if (!a->audio_started || start < a->abuf_start || start > end + out_rate) {
        if (audio_reposition(a, out_rate, start, err, errlen) < 0) return -1;
    }
    int produced = 0;
    while (produced < frames) {
        int64_t want = start + produced;
        int64_t hi = a->abuf_start + a->abuf_len;
        if (want < a->abuf_start) {
            /* gap before the first decoded sample: leave zeros */
            int64_t gap = a->abuf_start - want;
            if (gap > frames - produced) gap = frames - produced;
            produced += (int)gap;
            continue;
        }
        if (want < hi) {
            int64_t off = want - a->abuf_start;
            int n = (int)(hi - want);
            if (n > frames - produced) n = frames - produced;
            memcpy(dst + (size_t)produced * 2, a->abuf + (size_t)off * 2, (size_t)n * 2 * sizeof(float));
            produced += n;
            continue;
        }
        int r = audio_refill(a, out_rate, err, errlen);
        if (r < 0) return r;
        if (r == 1) break;
    }
    /* Keep only the history window behind the read cursor. */
    int64_t keep = start - ARC_AUDIO_HISTORY;
    if (keep > a->abuf_start) {
        int64_t last = a->abuf_start + a->abuf_len;
        if (keep > last) keep = last;
        int drop = (int)(keep - a->abuf_start);
        if (drop > 0) {
            memmove(a->abuf, a->abuf + (size_t)drop * 2,
                    (size_t)(a->abuf_len - drop) * 2 * sizeof(float));
            a->abuf_start += drop;
            a->abuf_len -= drop;
        }
    }
    return produced;
}