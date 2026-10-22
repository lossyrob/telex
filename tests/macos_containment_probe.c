#include <errno.h>
#include <stdio.h>
#include <sys/event.h>
#include <unistd.h>

int main(void) {
    const struct {
        const char *name;
        unsigned int notes;
        int expected;
    } probes[] = {
        {"NOTE_EXIT", NOTE_EXIT, 0},
        {"NOTE_FORK", NOTE_FORK, 0},
        {"NOTE_TRACK", NOTE_TRACK, ENOTSUP},
        {"NOTE_TRACKERR", NOTE_TRACKERR, ENOTSUP},
        {"NOTE_CHILD", NOTE_CHILD, ENOTSUP},
        {"NOTE_EXIT|NOTE_FORK|NOTE_TRACK", NOTE_EXIT | NOTE_FORK | NOTE_TRACK, ENOTSUP},
    };
    int failed = 0;

    for (size_t i = 0; i < sizeof(probes) / sizeof(probes[0]); ++i) {
        int queue = kqueue();
        if (queue == -1) {
            perror("kqueue");
            return 1;
        }
        struct kevent change;
        struct kevent receipt = {0};
        const struct timespec timeout = {0, 0};
        EV_SET(&change, getpid(), EVFILT_PROC, EV_ADD | EV_ENABLE | EV_RECEIPT,
               probes[i].notes, 0, NULL);
        errno = 0;
        int count = kevent(queue, &change, 1, &receipt, 1, &timeout);
        int call_errno = errno;
        int receipt_valid = count == 1 && (receipt.flags & EV_ERROR) != 0;
        int observed = count == -1 ? call_errno : (receipt_valid ? (int)receipt.data : -1);
        int matched = observed == probes[i].expected;
        printf("{\"probe\":\"kqueue\",\"notes\":\"%s\",\"note_bits\":%u,"
               "\"count\":%d,\"call_errno\":%d,\"receipt_flags\":%u,"
               "\"receipt_data\":%lld,\"expected_error\":%d,\"matched\":%s}\n",
               probes[i].name, probes[i].notes, count, call_errno,
               (unsigned int)receipt.flags, (long long)receipt.data,
               probes[i].expected, matched ? "true" : "false");
        failed |= !matched;
        if (close(queue) == -1) {
            perror("close kqueue");
            return 1;
        }
    }
    return failed ? 1 : 0;
}
