/* Linux experiment shim: explicitly armed, exact private read-only path.
 * No sleep or write interception. Corruption changes a returned read buffer,
 * never the file. One calling thread/one selected descriptor per experiment. */
#define _GNU_SOURCE
#include <dlfcn.h>
#include <errno.h>
#include <fcntl.h>
#include <limits.h>
#include <stdarg.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <time.h>
#include <unistd.h>

static _Thread_local struct {
    int active, fd, overflow;
    unsigned calls, successes, closes, corruptions, corrupt_opens;
    uint64_t offset;
    char path[PATH_MAX], schedule[256], events[16000], result[17000];
    size_t used;
} state = {.fd = -1};

static void event(const char *op, long result, int error) {
    struct timespec ts;
    clock_gettime(CLOCK_MONOTONIC, &ts);
    int n = snprintf(state.events + state.used, sizeof(state.events) - state.used,
        "%s{\"op\":\"%s\",\"result\":%ld,\"errno\":%d,\"ns\":%llu}",
        state.used ? "," : "", op, result, error,
        (unsigned long long)ts.tv_sec * 1000000000ULL + ts.tv_nsec);
    if (n < 0 || (size_t)n >= sizeof(state.events) - state.used) state.overflow = 1;
    else state.used += (size_t)n;
}

int straw_lookup_begin(const char *path, const char *schedule, uint64_t offset, unsigned corrupt_opens) {
    if (state.active || !path || path[0] != '/' || strlen(path) >= sizeof(state.path)
        || !schedule || strlen(schedule) >= sizeof(state.schedule)) return -1;
    for (const char *p = schedule; *p; ++p) if (!strchr("PNEAQFI", *p)) return -1;
    memset(&state, 0, sizeof(state));
    state.fd = -1; state.active = 1; state.offset = offset; state.corrupt_opens = corrupt_opens;
    strcpy(state.path, path); strcpy(state.schedule, schedule);
    return 0;
}

const char *straw_lookup_end(void) {
    state.active = 0;
    snprintf(state.result, sizeof(state.result),
        "{\"calls\":%u,\"successes\":%u,\"closes\":%u,\"corruptions\":%u,"
        "\"open_fd\":%d,\"overflow\":%d,\"events\":[%s]}",
        state.calls, state.successes, state.closes, state.corruptions,
        state.fd, state.overflow, state.events);
    return state.result;
}

static int selected(const char *path, int flags) {
    return state.active && path && !strcmp(path, state.path)
        && (flags & O_ACCMODE) == O_RDONLY && !(flags & O_CREAT);
}
static int injected(void) {
    char mode = state.calls < strlen(state.schedule) ? state.schedule[state.calls] : 'P';
    ++state.calls;
    int error = mode == 'N' ? ENOENT : mode == 'E' ? EIO : mode == 'A' ? EACCES
        : mode == 'Q' ? EDQUOT : mode == 'F' ? ENOSPC : mode == 'I' ? EINTR : 0;
    if (error) { event("open", -1, error); errno = error; }
    return error;
}
static int acquired(int fd) {
    int saved = errno;
    if (fd >= 0) {
        if (state.fd >= 0) state.overflow = 1;
        state.fd = fd; ++state.successes;
    }
    event("open", fd, fd < 0 ? saved : 0);
    errno = saved;
    return fd;
}
#define OPEN_WRAPPER(name) \
int name(const char *path, int flags, ...) { \
    int (*real_call)(const char *, int, ...) = dlsym(RTLD_NEXT, #name); \
    mode_t mode = 0; \
    if ((flags & O_CREAT) || (flags & O_TMPFILE) == O_TMPFILE) { \
        va_list args; va_start(args, flags); mode = va_arg(args, mode_t); va_end(args); \
    } \
    int target = selected(path, flags); \
    if (target && injected()) return -1; \
    int fd = real_call(path, flags, mode); \
    return target ? acquired(fd) : fd; \
}
OPEN_WRAPPER(open)
OPEN_WRAPPER(open64)

ssize_t read(int fd, void *buffer, size_t count) {
    ssize_t (*real_read)(int, void *, size_t) = dlsym(RTLD_NEXT, "read");
    off64_t (*real_seek)(int, off64_t, int) = dlsym(RTLD_NEXT, "lseek64");
    int target = state.active && fd == state.fd;
    off64_t position = target ? real_seek(fd, 0, SEEK_CUR) : -1;
    ssize_t n = real_read(fd, buffer, count);
    int saved = errno;
    if (target && n > 0 && position >= 0 && state.successes <= state.corrupt_opens
        && state.offset >= (uint64_t)position && state.offset - (uint64_t)position < (uint64_t)n) {
        ((unsigned char *)buffer)[state.offset - (uint64_t)position] ^= 1;
        ++state.corruptions; event("corrupt", (long)state.offset, 0);
    }
    errno = saved;
    return n;
}
int close(int fd) {
    int (*real_close)(int) = dlsym(RTLD_NEXT, "close");
    int target = state.active && fd == state.fd;
    int result = real_close(fd), saved = errno;
    if (target) {
        ++state.closes; state.fd = -1;
        event("close", result, result < 0 ? saved : 0);
    }
    errno = saved;
    return result;
}
