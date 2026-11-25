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
#include <libavutil/imgutils.h>
#include <libavutil/opt.h>
#include <libswscale/swscale.h>
#include <libswresample/swresample.h>
#include <stdint.h>
#include <string.h>
#include <stdio.h>

typedef struct ArcAv {
    AVFormatContext *fmt;
    AVCodecContext *dec;
    AVPacket *pkt;
    AVFrame *frame;
    AVFrame *cur;         /* last delivered frame */
    AVFrame *held;        /* look-ahead: first decoded frame after `cur` */
    int drained;          /* decoder returned EOF */
    int stream;
    int is_audio;
    struct SwsContext *sws;
    int sws_w, sws_h, sws_fmt;
    struct SwrContext *swr;
    int out_rate;
    int64_t last_pts;     /* pts of the last delivered video frame, INT64_MIN if none */
    int eof_sent;
    /* buffered resampled audio (interleaved stereo f32) */
    float *abuf;
    int abuf_len, abuf_cap, abuf_pos;
    int64_t audio_next_sample; /* output-rate sample index of abuf[abuf_pos] */
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
    ArcAv *a = av_mallocz(sizeof *a);
    if (!a) { set_err(err, errlen, "out of memory", 0); return -1; }
    a->last_pts = INT64_MIN;
    a->is_audio = kind == 1;
    int r = avformat_open_input(&a->fmt, path, NULL, NULL);
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
        r = av_read_frame(a->fmt, a->pkt);
        if (r == AVERROR_EOF) {
            avcodec_send_packet(a->dec, NULL);
            a->eof_sent = 1;
            continue;
        }
        if (r < 0) { set_err(err, errlen, "read_frame", r); return -1; }
        if (a->pkt->stream_index == a->stream) {
            r = avcodec_send_packet(a->dec, a->pkt);
            if (r < 0 && r != AVERROR(EAGAIN) && r != AVERROR_INVALIDDATA) {
                av_packet_unref(a->pkt);
                set_err(err, errlen, "send_packet", r);
                return -1;
            }
        }
        av_packet_unref(a->pkt);
    }
}

static int64_t frame_pts(const AVFrame *f) {
    return f->best_effort_timestamp != AV_NOPTS_VALUE ? f->best_effort_timestamp : f->pts;
}

static int to_rgba(ArcAv *a, const AVFrame *f, uint8_t *dst, int stride) {
    if (!a->sws || a->sws_w != f->width || a->sws_h != f->height || a->sws_fmt != f->format) {
        if (a->sws) sws_freeContext(a->sws);
        a->sws = sws_getContext(f->width, f->height, f->format, f->width, f->height, AV_PIX_FMT_RGBA,
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
    return sws_scale(a->sws, (const uint8_t *const *)f->data, f->linesize, 0, f->height, planes, strides) > 0 ? 0 : -1;
}

/*
 * Frame-accurate video decode: deliver the frame whose interval contains `target_pts`
 * (the last frame with pts <= target). Sequential and repeated requests are served without
 * seeking; backward or far-forward requests seek to the preceding keyframe and decode forward.
 * `dst` must hold width*height*4 bytes. `got_pts` receives the delivered frame's pts.
 */
int arc_av_video_at(ArcAv *a, int64_t target_pts, uint8_t *dst, int stride, int64_t *got_pts, char *err, int errlen) {
    if (a->is_audio) { set_err(err, errlen, "not a video decoder", 0); return -1; }
    AVStream *s = a->fmt->streams[a->stream];
    int have_cur = a->cur->buf[0] != NULL;
    int have_held = a->held->buf[0] != NULL;
    int64_t cur_pts = have_cur ? frame_pts(a->cur) : INT64_MIN;
    int64_t held_pts = have_held ? frame_pts(a->held) : INT64_MAX;

    /* 1. the current frame still covers the target */
    if (have_cur && cur_pts <= target_pts && (have_held ? target_pts < held_pts : a->drained)) {
        if (to_rgba(a, a->cur, dst, stride) < 0) { set_err(err, errlen, "convert", 0); return -1; }
        *got_pts = cur_pts;
        return 0;
    }
    /* 2. seek if going backwards or far ahead */
    int64_t ahead_limit = av_rescale_q(2 * AV_TIME_BASE, AV_TIME_BASE_Q, s->time_base);
    int64_t pos = have_cur ? cur_pts : (have_held ? held_pts : INT64_MIN);
    int need_seek = (!have_cur && !have_held) || target_pts < pos || target_pts - pos > ahead_limit;
    if (need_seek) {
        int r = av_seek_frame(a->fmt, a->stream, target_pts, AVSEEK_FLAG_BACKWARD);
        if (r < 0) { set_err(err, errlen, "seek", r); return -1; }
        avcodec_flush_buffers(a->dec);
        a->eof_sent = 0;
        a->drained = 0;
        av_frame_unref(a->cur);
        av_frame_unref(a->held);
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
    if (!have_cur) {
        if (!have_held) return 1;           /* no frames at all */
        av_frame_move_ref(a->cur, a->held); /* target precedes the first frame */
        have_held = 0;
    }
    if (to_rgba(a, a->cur, dst, stride) < 0) { set_err(err, errlen, "convert", 0); return -1; }
    *got_pts = frame_pts(a->cur);
    a->last_pts = *got_pts;
    return 0;
}

/* ---- audio ---------------------------------------------------------------------------------- */

static int ensure_swr(ArcAv *a, const AVFrame *f, int out_rate) {
    if (a->swr && a->out_rate == out_rate) return 0;
    if (a->swr) swr_free(&a->swr);
    AVChannelLayout stereo = AV_CHANNEL_LAYOUT_STEREO;
    AVChannelLayout in;
    if (av_channel_layout_copy(&in, &f->ch_layout) < 0 || in.nb_channels == 0) av_channel_layout_default(&in, 2);
    int r = swr_alloc_set_opts2(&a->swr, &stereo, AV_SAMPLE_FMT_FLT, out_rate, &in, f->format, f->sample_rate, 0, NULL);
    av_channel_layout_uninit(&in);
    if (r < 0) return r;
    /* high quality, deterministic resampler */
    av_opt_set_int(a->swr, "filter_size", 32, 0);
    av_opt_set_int(a->swr, "phase_shift", 10, 0);
    r = swr_init(a->swr);
    if (r < 0) return r;
    a->out_rate = out_rate;
    return 0;
}

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

/*
 * Read `frames` stereo f32 frames starting at output sample `start` (at `out_rate`), counted from
 * the stream's first sample. Missing samples (before start / after end) are zero.
 * Returns the number of frames that came from the stream (rest zero-filled).
 */
int arc_av_audio_read(ArcAv *a, int out_rate, int64_t start, int frames, float *dst, char *err, int errlen) {
    if (!a->is_audio) { set_err(err, errlen, "not an audio decoder", 0); return -1; }
    memset(dst, 0, (size_t)frames * 2 * sizeof(float));
    int64_t buf_start = a->audio_next_sample;
    int64_t buf_end = buf_start + (a->abuf_len - a->abuf_pos);
    if (a->swr == NULL || start < buf_start || start > buf_end + out_rate) {
        /* (re)position: seek to just before `start`, then decode forward and discard */
        AVStream *s = a->fmt->streams[a->stream];
        int64_t ts = av_rescale_q(start, (AVRational){1, out_rate}, s->time_base);
        if (s->start_time != AV_NOPTS_VALUE) ts += s->start_time;
        int r = av_seek_frame(a->fmt, a->stream, ts, AVSEEK_FLAG_BACKWARD);
        if (r < 0 && start > 0) { set_err(err, errlen, "audio seek", r); return -1; }
        avcodec_flush_buffers(a->dec);
        a->eof_sent = 0;
        a->abuf_len = a->abuf_pos = 0;
        a->audio_next_sample = -1; /* set from the first decoded frame */
        if (a->swr) swr_free(&a->swr);
    }
    int produced = 0;
    while (produced < frames) {
        int avail = a->abuf_len - a->abuf_pos;
        if (avail > 0 && a->audio_next_sample >= 0) {
            int64_t want = start + produced;
            if (a->audio_next_sample < want) {
                int64_t skip = want - a->audio_next_sample;
                if (skip > avail) skip = avail;
                a->abuf_pos += (int)skip;
                a->audio_next_sample += skip;
                continue;
            }
            if (a->audio_next_sample > want) {
                /* gap before stream data: leave zeros */
                int64_t gap = a->audio_next_sample - want;
                if (gap > frames - produced) gap = frames - produced;
                produced += (int)gap;
                continue;
            }
            int n = avail < frames - produced ? avail : frames - produced;
            memcpy(dst + (size_t)produced * 2, a->abuf + (size_t)a->abuf_pos * 2, (size_t)n * 2 * sizeof(float));
            a->abuf_pos += n;
            a->audio_next_sample += n;
            produced += n;
            continue;
        }
        /* refill */
        if (a->abuf_pos > 0) {
            memmove(a->abuf, a->abuf + (size_t)a->abuf_pos * 2, (size_t)(a->abuf_len - a->abuf_pos) * 2 * sizeof(float));
            a->abuf_len -= a->abuf_pos;
            a->abuf_pos = 0;
        }
        int r = next_frame(a, err, errlen);
        if (r < 0) return r;
        if (r == 1) break;
        if (ensure_swr(a, a->frame, out_rate) < 0) { set_err(err, errlen, "resampler", 0); return -1; }
        if (a->audio_next_sample < 0) {
            AVStream *s = a->fmt->streams[a->stream];
            int64_t pts = frame_pts(a->frame);
            int64_t base = s->start_time != AV_NOPTS_VALUE ? s->start_time : 0;
            a->audio_next_sample = pts == AV_NOPTS_VALUE ? 0 : av_rescale_q(pts - base, s->time_base, (AVRational){1, out_rate});
            if (a->audio_next_sample < 0) a->audio_next_sample = 0;
        }
        int maxo = swr_get_out_samples(a->swr, a->frame->nb_samples);
        if (abuf_push(a, maxo) < 0) { set_err(err, errlen, "out of memory", 0); return -1; }
        uint8_t *outp = (uint8_t *)(a->abuf + (size_t)a->abuf_len * 2);
        int got = swr_convert(a->swr, &outp, maxo, (const uint8_t **)a->frame->extended_data, a->frame->nb_samples);
        av_frame_unref(a->frame);
        if (got < 0) { set_err(err, errlen, "resample", got); return -1; }
        a->abuf_len += got;
    }
    return produced;
}
