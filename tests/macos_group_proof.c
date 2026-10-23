#define _DARWIN_C_SOURCE
#include <errno.h>
#include <poll.h>
#include <signal.h>
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/sysctl.h>
#include <sys/types.h>
#include <sys/wait.h>
#include <time.h>
#include <unistd.h>

/* Isolated syscall experiment, not a Watcher adapter or Rust type-safety proof. */
enum { GATED = 1, RELEASED, READY, FORK_ENTER, BORN, RESPONSE, COOPERATIVE_EXIT };
typedef struct {
    int kind, value;
    pid_t pid, pgid, ppid;
    uint64_t time_ns;
} Event;
typedef struct { pid_t leader; } Anchor;
typedef struct { pid_t pgid; } Observation;
typedef struct { Anchor *anchor; int events, control, fork_gate; } Fixture;
static int phase; /* Instrumentation: 0 anchored, 1 sealed, 2 exactly reaped. */
static unsigned nonzero_calls;
static const char *executable;

static uint64_t now_ns(void) {
    struct timespec ts;
    if (clock_gettime(CLOCK_MONOTONIC, &ts) != 0) _exit(110);
    return (uint64_t)ts.tv_sec * 1000000000ULL + (uint64_t)ts.tv_nsec;
}

static void record(const char *event, pid_t subject, int value, int rc, int error) {
    char line[768];
    int n = snprintf(line, sizeof(line),
        "{\"event\":\"%s\",\"emitter\":%d,\"emitter_pgid\":%d,"
        "\"subject\":%d,\"value\":%d,\"rc\":%d,\"errno\":%d,"
        "\"phase\":%d,\"nonzero_calls\":%u,\"time_ns\":%llu}\n",
        event, getpid(), getpgrp(), subject, value, rc, error, phase,
        nonzero_calls, (unsigned long long)now_ns());
    if (n < 0 || (size_t)n >= sizeof(line) ||
        write(STDOUT_FILENO, line, (size_t)n) != n) _exit(111);
}

static void require(int ok, const char *event) {
    if (!ok) {
        record(event, 0, 0, -1, errno);
        exit(1);
    }
}

static void pause_ms(int ms) {
    struct timespec ts = {ms / 1000, (ms % 1000) * 1000000L};
    while (nanosleep(&ts, &ts) < 0 && errno == EINTR) {}
}

static void pipe_checked(int fds[2]) {
    require(pipe(fds) == 0, "pipe_failed");
}

static int byte_read(int fd, char *byte) {
    ssize_t n;
    do { n = read(fd, byte, 1); } while (n < 0 && errno == EINTR);
    require(n >= 0, "private_read_failed");
    return (int)n;
}

static void send_event(int fd, int kind, int value) {
    Event e = {kind, value, getpid(), getpgrp(), getppid(), now_ns()};
    require(write(fd, &e, sizeof(e)) == (ssize_t)sizeof(e), "event_write_failed");
}

/* 1 event, 0 bounded wait, -1 EOF; no process-table or numeric-PID liveness query. */
static int receive_event(int fd, int timeout_ms, Event *e) {
    struct pollfd p = {fd, POLLIN, 0};
    int rc;
    do { rc = poll(&p, 1, timeout_ms); } while (rc < 0 && errno == EINTR);
    require(rc >= 0, "event_poll_failed");
    if (!rc) return 0;
    ssize_t n;
    do { n = read(fd, e, sizeof(*e)); } while (n < 0 && errno == EINTR);
    if (!n) return -1;
    require(n == (ssize_t)sizeof(*e), "event_frame_failed");
    char line[768];
    int size = snprintf(line, sizeof(line),
        "{\"event\":\"fixture_event\",\"kind\":%d,\"pid\":%d,\"pgid\":%d,"
        "\"ppid\":%d,\"value\":%d,\"emitted_ns\":%llu,\"received_ns\":%llu,"
        "\"phase\":%d}\n", e->kind, e->pid, e->pgid, e->ppid, e->value,
        (unsigned long long)e->time_ns, (unsigned long long)now_ns(), phase);
    require(size > 0 && (size_t)size < sizeof(line), "event_format_failed");
    require(write(STDOUT_FILENO, line, (size_t)size) == size, "event_log_failed");
    return 1;
}

static void wait_kind(int fd, int kind, int count, pid_t pgid) {
    uint64_t end = now_ns() + 5000000000ULL;
    while (count && now_ns() < end) {
        Event e;
        int rc = receive_event(fd, 100, &e);
        require(rc >= 0, "premature_fixture_eof");
        if (rc == 1) {
            require(e.pgid == pgid, "fixture_escaped_group");
            if (e.kind == kind) --count;
        }
    }
    require(!count, "fixture_barrier_timeout");
}

static void cooperative_leaf(int events, int control, int ready_kind) {
    alarm(20);
    signal(SIGTERM, SIG_IGN);
    signal(SIGHUP, SIG_IGN);
    send_event(events, ready_kind, 0);
    char byte;
    while (byte_read(control, &byte)) send_event(events, RESPONSE, byte);
    send_event(events, COOPERATIVE_EXIT, 0);
    _exit(0);
}

static void exec_leaf(int events, int control) {
    char e[24], c[24];
    snprintf(e, sizeof(e), "%d", events);
    snprintf(c, sizeof(c), "%d", control);
    execl(executable, executable, "leaf", e, c, (char *)NULL);
    require(0, "fixture_exec_failed");
}

static void detector(const char *scenario, int events, int control, int fork_gate) {
    alarm(20);
    signal(SIGTERM, SIG_IGN);
    signal(SIGHUP, SIG_IGN);
    if (!strcmp(scenario, "zombie")) {
        send_event(events, READY, 0);
        _exit(0);
    }
    if (!strncmp(scenario, "race", 4)) {
        for (int i = 0; i < 4; ++i) {
            pid_t worker = fork();
            require(worker >= 0, "worker_fork_failed");
            if (worker == 0) {
                alarm(20);
                send_event(events, READY, i);
                char byte;
                if (!byte_read(fork_gate, &byte)) _exit(0);
                require(byte == 'f', "bad_fork_release");
                send_event(events, FORK_ENTER, i);
                pid_t child = fork();
                require(child >= 0, "racing_fork_failed");
                if (!child) cooperative_leaf(events, control, BORN);
                int status;
                require(waitpid(child, &status, 0) == child, "worker_reap_failed");
                _exit(0);
            }
        }
        cooperative_leaf(events, control, READY);
    }
    int depth = !strcmp(scenario, "multilevel") ? 3 : 1;
    pid_t child = fork();
    require(child >= 0, "ordinary_fork_failed");
    if (!child) {
        for (int level = 1; level < depth; ++level) {
            child = fork();
            require(child >= 0, "multilevel_fork_failed");
            if (child) cooperative_leaf(events, control, READY);
        }
        exec_leaf(events, control);
    }
    if (!strcmp(scenario, "leader-exit")) {
        send_event(events, READY, 0);
        _exit(0);
    }
    cooperative_leaf(events, control, READY);
}

static Fixture prepare(const char *scenario) {
    int gate[2], events[2], control[2], fork_gate[2];
    pipe_checked(gate);
    pipe_checked(events);
    pipe_checked(control);
    pipe_checked(fork_gate);
    pid_t leader = fork();
    require(leader >= 0, "leader_fork_failed");
    if (!leader) {
        alarm(20);
        close(gate[1]); close(events[0]); close(control[1]); close(fork_gate[1]);
        close(STDIN_FILENO);
        require(setpgid(0, 0) == 0, "leader_setpgid_failed");
        send_event(events[1], GATED, 0);
        char byte;
        if (!byte_read(gate[0], &byte)) _exit(0);
        require(byte == 'r', "bad_detector_release");
        close(gate[0]);
        send_event(events[1], RELEASED, 0);
        detector(scenario, events[1], control[0], fork_gate[0]);
        _exit(112);
    }
    close(gate[0]); close(events[1]); close(control[0]); close(fork_gate[0]);
    wait_kind(events[0], GATED, 1, leader);
    require(leader != getpgrp() && getpgid(leader) == leader, "bad_owned_anchor");
    record("prepared_unreleased_owned_group", leader, leader, 0, 0);
    Anchor *anchor = malloc(sizeof(*anchor));
    require(anchor != NULL, "anchor_allocation_failed");
    anchor->leader = leader;
    require(write(gate[1], "r", 1) == 1, "release_write_failed");
    close(gate[1]);
    record("release_sent", leader, 0, 0, 0);
    Fixture f = {anchor, events[0], control[1], fork_gate[1]};
    return f;
}

static siginfo_t exited_wnowait(pid_t leader) {
    uint64_t end = now_ns() + 5000000000ULL;
    siginfo_t info;
    for (;;) {
        memset(&info, 0, sizeof(info));
        errno = 0;
        int rc = waitid(P_PID, (id_t)leader, &info, WEXITED | WNOHANG | WNOWAIT);
        int error = errno;
        if (rc != 0 || info.si_pid) {
            record("waitid_WNOWAIT", info.si_pid, info.si_code, rc, error);
            require(rc == 0 && info.si_pid == leader, "exclusive_wait_ownership_lost");
            require(info.si_code == CLD_EXITED || info.si_code == CLD_KILLED ||
                    info.si_code == CLD_DUMPED, "ambiguous_leader_exit");
            record("WNOWAIT_exit_status", info.si_pid, info.si_status, rc, error);
            return info;
        }
        require(now_ns() < end, "leader_exit_timeout");
        pause_ms(1);
    }
}

/* All candidate kill calls pass this audit point. No other call site uses kill. */
static int audited_kill(pid_t pgid, int sig, int *error) {
    require(pgid > 1 && pgid != getpgrp(), "invalid_signal_target");
    require(sig == 0 || phase == 0, "NONZERO_AFTER_SEAL");
    if (sig) ++nonzero_calls;
    errno = 0;
    int rc = kill(-pgid, sig);
    *error = errno;
    record(sig ? "nonzero_group_signal" : "libc_group_observation",
           pgid, sig, rc, *error);
    return rc;
}

static int anchor_signal(Anchor *anchor, int sig, int *error) {
    if (!anchor || phase != 0) {
        record("sealed_signal_attempt_rejected", anchor ? anchor->leader : 0, sig, -1, 0);
        return -2;
    }
    return audited_kill(anchor->leader, sig, error);
}

static Observation seal(Anchor **anchor) {
    require(phase == 0 && *anchor != NULL, "invalid_seal");
    Observation observation = {(*anchor)->leader};
    memset(*anchor, 0, sizeof(**anchor));
    free(*anchor);
    *anchor = NULL;
    phase = 1;
    record("signal_authority_consumed", observation.pgid, 0, 0, 0);
    return observation;
}

static void exact_reap(Observation observation) {
    siginfo_t info = exited_wnowait(observation.pgid);
    int status = 0;
    errno = 0;
    pid_t waited = waitpid(observation.pgid, &status, 0);
    int error = errno;
    record("exact_waitpid", waited, status, (int)waited, error);
    require(phase == 1 && waited == observation.pgid &&
            (WIFEXITED(status) || WIFSIGNALED(status)), "exact_reap_failed");
    require((info.si_code == CLD_EXITED && WIFEXITED(status) &&
             WEXITSTATUS(status) == info.si_status) ||
            (info.si_code != CLD_EXITED && WIFSIGNALED(status) &&
             WTERMSIG(status) == info.si_status), "WNOWAIT_reap_evidence_mismatch");
    phase = 2;
    record("leader_exactly_reaped", waited, status, 0, 0);
    memset(&info, 0, sizeof(info));
    errno = 0;
    int rc = waitid(P_PID, (id_t)waited, &info, WEXITED | WNOHANG | WNOWAIT);
    error = errno;
    record("post_reap_ECHILD", waited, 0, rc, error);
    require(rc == -1 && error == ECHILD, "reap_ownership_check_failed");
}

static void challenge_seal(Observation observation) {
    const char *paths[] = {"timeout", "retry", "cancellation", "error", "Drop",
                          "guard_shutdown", "restart", "finalization"};
    unsigned before = nonzero_calls;
    for (size_t i = 0; i < sizeof(paths) / sizeof(paths[0]); ++i) {
        int error = 0;
        Anchor reconstructed = {observation.pgid};
        record(paths[i], observation.pgid, 0, 0, 0);
        require(anchor_signal(&reconstructed, SIGKILL, &error) == -2,
                "numeric_reconstruction_allowed");
    }
    require(before == nonzero_calls, "sealed_syscall_count_changed");
    record("seal_instrumentation_passed_not_Rust_type_proof", observation.pgid, 8, 0, 0);
}

static int observe(Observation observation) {
    int error, rc;
    do { rc = audited_kill(observation.pgid, 0, &error); } while (rc < 0 && error == EINTR);
    if (rc == -1 && error == ESRCH) return 1;
    if (rc == 0 || (rc == -1 && error == EPERM)) return 0;
    record("observation_uncertainty", observation.pgid, 0, rc, error);
    return -1;
}

static void fixture_eof(Fixture *f, Observation observation) {
    close(f->control); close(f->fork_gate);
    uint64_t end = now_ns() + 22000000000ULL;
    int eof = 0;
    while (!eof && now_ns() < end) {
        Event e;
        eof = receive_event(f->events, 100, &e) == -1;
    }
    close(f->events);
    record("private_teardown_event_eof", observation.pgid, eof, 0, 0);
    require(eof, "finite_fixture_teardown_failed");
    end = now_ns() + 4000000000ULL;
    int absent;
    do {
        absent = observe(observation);
        if (absent != 0) break;
        pause_ms(10);
    } while (now_ns() < end);
    require(absent == 1, "cleanup_absence_unproven");
}

static int run_case(const char *scenario, int skew_us) {
    Fixture f = prepare(scenario);
    pid_t leader = f.anchor->leader;
    int ready = !strncmp(scenario, "race", 4) ? 5 :
                !strcmp(scenario, "multilevel") ? 4 : !strcmp(scenario, "zombie") ? 1 : 2;
    wait_kind(f.events, READY, ready, leader);
    if (!strcmp(scenario, "leader-exit") || !strcmp(scenario, "zombie")) {
        (void)exited_wnowait(leader);
        (void)exited_wnowait(leader);
    }
    if (!strcmp(scenario, "runtime-eof")) {
        record("awaiting_runtime_liveness_EOF", leader, 0, 0, 0);
        char byte;
        require(byte_read(STDIN_FILENO, &byte) == 0, "expected_runtime_EOF");
        record("runtime_liveness_EOF", leader, 0, 0, 0);
    }
    if (!strcmp(scenario, "ignored-signals")) {
        int error;
        require(anchor_signal(f.anchor, SIGTERM, &error) == 0, "sigterm_failed");
        require(anchor_signal(f.anchor, SIGHUP, &error) == 0, "sighup_failed");
        require(write(f.control, "i", 1) == 1, "ignore_challenge_write_failed");
        wait_kind(f.events, RESPONSE, 1, leader);
    }
    if (!strcmp(scenario, "race-born-first") || !strcmp(scenario, "race-overlap")) {
        record("fork_release_barrier", leader, skew_us, 0, 0);
        require(write(f.fork_gate, "ffff", 4) == 4, "fork_release_failed");
        if (!strcmp(scenario, "race-born-first")) wait_kind(f.events, BORN, 4, leader);
        else if (skew_us) {
            struct timespec delay = {0, skew_us * 1000L};
            require(nanosleep(&delay, NULL) == 0, "race_skew_failed");
        }
    }
    int error = 0;
    int rc = anchor_signal(f.anchor, SIGKILL, &error);
    require(rc == 0 || (rc == -1 && error == EPERM), "final_group_signal_failed");
    record(rc == 0 ? "SIGKILL_accepted" : "EPERM_not_success_source_gate_unresolved",
           leader, 0, rc, error);
    Observation observation = seal(&f.anchor);
    exact_reap(observation);
    challenge_seal(observation);

    int first_absent = observe(observation);
    int survivor = 0, eof = 0;
    record("post_seal_fresh_challenge", leader, 'Z', 0, 0);
    errno = 0;
    ssize_t wrote = write(f.control, "ZZZZ", 4);
    int write_error = errno;
    record("challenge_write", leader, (int)wrote, (int)wrote, write_error);
    require(wrote == 4 || (wrote == -1 && write_error == EPIPE), "challenge_write_failed");
    uint64_t end = now_ns() + 100000000ULL;
    while (!eof && now_ns() < end) {
        Event e;
        int event_rc = receive_event(f.events, 10, &e);
        eof = event_rc == -1;
        if (event_rc == 1 && e.kind == RESPONSE && e.value == 'Z') {
            require(e.pgid == leader, "survivor_group_mismatch");
            survivor = 1;
            record("first_post_seal_live_response", e.pid, first_absent, 0, 0);
        }
    }
    if (survivor) {
        /* A second fresh challenge distinguishes sustained survival from delivery latency. */
        pause_ms(1000);
        record("sustained_survival_challenge", leader, 'Q', 0, 0);
        errno = 0;
        wrote = write(f.control, "QQQQ", 4);
        write_error = errno;
        record("sustained_challenge_write", leader, (int)wrote, (int)wrote, write_error);
        require(wrote == 4 || (wrote == -1 && write_error == EPIPE),
                "sustained_challenge_failed");
        survivor = 0;
        end = now_ns() + 500000000ULL;
        while (!eof && now_ns() < end) {
            Event e;
            int event_rc = receive_event(f.events, 10, &e);
            eof = event_rc == -1;
            if (event_rc == 1 && e.kind == RESPONSE && e.value == 'Q') {
                require(e.pgid == leader, "sustained_survivor_group_mismatch");
                survivor = 1;
                record("LIVE_IN_GROUP_AFTER_SEAL", e.pid, first_absent, 0, 0);
            }
        }
        if (first_absent == 1) {
            record("ABSENCE_PRECEDED_OWNED_LIVE_RESPONSE", leader, 0, 0, 0);
            survivor = 1;
        }
        if (!survivor) record("transient_response_not_sustained", leader, 0, 0, 0);
        (void)observe(observation);
    }
    int absence = first_absent;
    end = now_ns() + 4000000000ULL;
    while (!survivor && absence == 0 && now_ns() < end) {
        pause_ms(10);
        absence = observe(observation);
    }
    record("candidate_outcome", leader, survivor ? 42 : absence == 1 ? 0 : 2, rc, error);
    fixture_eof(&f, observation);
    if (survivor) return 42;
    require(absence == 1, "candidate_cleanup_blocked");
    return 0;
}

/* Every churn child is direct, unreaped, gated, and reaped exactly; no orphan kills. */
static int churn_batch(int count, pid_t target, Observation *observation) {
    int ready[2], control[2];
    pipe_checked(ready); pipe_checked(control);
    pid_t children[8];
    for (int i = 0; i < count; ++i) {
        children[i] = fork();
        require(children[i] >= 0, "churn_fork_failed");
        if (!children[i]) {
            alarm(15);
            close(ready[0]); close(control[1]);
            require(setpgid(0, 0) == 0, "churn_setpgid_failed");
            pid_t identity[2] = {getpid(), getpgrp()};
            require(write(ready[1], identity, sizeof(identity)) == (ssize_t)sizeof(identity),
                    "churn_ready_failed");
            close(ready[1]);
            char byte;
            require(byte_read(control[0], &byte) == 0, "churn_control_not_EOF");
            _exit(0);
        }
    }
    close(ready[1]); close(control[0]);
    int found = 0;
    for (int i = 0; i < count; ++i) {
        pid_t identity[2];
        require(read(ready[0], identity, sizeof(identity)) == (ssize_t)sizeof(identity),
                "churn_identity_failed");
        require(identity[0] == identity[1], "churn_not_new_group");
        if (identity[0] == target) {
            require(observation != NULL, "PID_REUSED_WHILE_ANCHORED");
            record("ACTUAL_PID_PGID_REUSE", target, count, 0, 0);
            require(observe(*observation) == 0, "replacement_not_conservative_presence");
            challenge_seal(*observation);
            found = 1;
        }
    }
    close(ready[0]); close(control[1]);
    for (int i = 0; i < count; ++i) {
        int status;
        require(waitpid(children[i], &status, 0) == children[i] &&
                WIFEXITED(status) && WEXITSTATUS(status) == 0, "churn_exact_reap_failed");
    }
    return found;
}

static int reuse_probe(void) {
    uint64_t start = now_ns(), end = start + 580000000000ULL;
    Fixture f = prepare("zombie");
    pid_t target = f.anchor->leader;
    wait_kind(f.events, READY, 1, target);
    (void)exited_wnowait(target);
    int created = 0;
    for (; created < 1024; created += 8) churn_batch(8, target, NULL);
    record("anchored_no_reuse_sample", target, created, 0, 0);
    int error;
    int rc = anchor_signal(f.anchor, SIGKILL, &error);
    require(rc == 0 || (rc == -1 && error == EPERM), "reuse_final_signal_failed");
    Observation observation = seal(&f.anchor);
    exact_reap(observation);
    require(observe(observation) == 1, "reuse_original_group_not_absent");
    fixture_eof(&f, observation);
    int found = 0;
    /* 119999 churn forks plus the target leader: <=120000 creations. */
    while (!found && created < 119999 && now_ns() < end) {
        int batch = 119999 - created < 8 ? 119999 - created : 8;
        found = churn_batch(batch, target, &observation);
        created += batch;
        if (created % 4096 == 0) record("churn_progress", target, created, 0, 0);
    }
    require(observe(observation) == 1, "replacement_cleanup_not_absent");
    record(found ? "reuse_observed" : "reuse_INCONCLUSIVE", target, created,
           (int)((now_ns() - start) / 1000000000ULL), 0);
    return 0;
}

static void loss_leaf(int control) {
    alarm(20);
    record("loss_detector_exec_entered", getpid(), getpgrp(), 0, 0);
    char byte;
    while (byte_read(control, &byte)) record("loss_live_response", getpid(), byte, 0, 0);
    record("loss_cooperative_exit", getpid(), 0, 0, 0);
    _exit(0);
}

static int loss_guard(const char *when, int cooperative) {
    int gate[2], events[2];
    pipe_checked(gate); pipe_checked(events);
    pid_t leader = fork();
    require(leader >= 0, "loss_fork_failed");
    if (!leader) {
        alarm(20);
        close(gate[1]); close(events[0]); close(STDIN_FILENO);
        require(setpgid(0, 0) == 0, "loss_setpgid_failed");
        send_event(events[1], GATED, 0);
        char byte;
        if (!byte_read(gate[0], &byte)) {
            record("guard_loss_before_release_no_detector", getpid(), getpgrp(), 0, 0);
            _exit(0);
        }
        require(byte == 'r', "loss_bad_release");
        close(gate[0]);
        send_event(events[1], RELEASED, 0);
        close(events[1]);
        /* Hold after release but before exec until the observer reaps the dead guard. */
        require(byte_read(cooperative, &byte) == 1 && byte == 'e', "loss_exec_barrier_failed");
        char control[24];
        snprintf(control, sizeof(control), "%d", cooperative);
        execl(executable, executable, "loss-leaf", control, (char *)NULL);
        require(0, "loss_exec_failed");
    }
    close(gate[0]); close(events[1]); close(cooperative);
    wait_kind(events[0], GATED, 1, leader);
    require(getpgid(leader) == leader && leader != getpgrp(), "loss_anchor_failed");
    if (!strcmp(when, "after")) {
        require(write(gate[1], "r", 1) == 1, "loss_release_failed");
        wait_kind(events[0], RELEASED, 1, leader);
    }
    record("guard_loss_barrier", leader, !strcmp(when, "after"), 0, 0);
    char byte;
    require(byte_read(STDIN_FILENO, &byte) == 1 && byte == 'x', "loss_death_request_failed");
    record("intentional_guard_exit_no_receipt", leader, 99, 0, 0);
    _exit(99);
}

int main(int argc, char **argv) {
    require(argc >= 2, "missing_mode");
    executable = argv[0];
    require(executable[0] == '/', "binary_must_be_absolute");
    require(getuid() != 0, "root_execution_forbidden");
    signal(SIGPIPE, SIG_IGN);
    if (!strcmp(argv[1], "leaf")) {
        require(argc == 4, "leaf_arguments");
        cooperative_leaf(atoi(argv[2]), atoi(argv[3]), READY);
    }
    if (!strcmp(argv[1], "loss-leaf")) {
        require(argc == 3, "loss_leaf_arguments");
        loss_leaf(atoi(argv[2]));
    }
    if (!strcmp(argv[1], "sentinel")) {
        alarm(900);
        char byte;
        while (byte_read(STDIN_FILENO, &byte))
            record("sentinel_response", getpid(), byte, 0, 0);
        record("sentinel_cooperative_exit", getpid(), 0, 0, 0);
        return 0;
    }
    require(setsid() == getpid(), "private_guard_setsid_failed");
    struct sigaction action;
    require(sigaction(SIGCHLD, NULL, &action) == 0 &&
            action.sa_handler != SIG_IGN && !(action.sa_flags & SA_NOCLDWAIT),
            "automatic_reaping_forbidden");
    char uuid[128] = {0};
    size_t size = sizeof(uuid);
    require(sysctlbyname("kern.bootsessionuuid", uuid, &size, NULL, 0) == 0 &&
            size > 1 && size <= sizeof(uuid) && memchr(uuid, 0, size) != NULL,
            "boot_session_uuid_unavailable");
    char line[256];
    int n = snprintf(line, sizeof(line), "{\"event\":\"boot_session\",\"uuid\":\"%s\"}\n", uuid);
    require(n > 0 && (size_t)n < sizeof(line) &&
            write(STDOUT_FILENO, line, (size_t)n) == n, "boot_uuid_record_failed");
    record("private_guard_no_auto_reap", getpid(), getsid(0), 0, 0);
    if (!strcmp(argv[1], "reuse")) {
        alarm(620);
        return reuse_probe();
    }
    alarm(60);
    if (!strcmp(argv[1], "loss-guard")) {
        require(argc == 4, "loss_arguments");
        return loss_guard(argv[2], atoi(argv[3]));
    }
    require(!strcmp(argv[1], "case") && argc >= 3, "case_arguments");
    return run_case(argv[2], argc == 4 ? atoi(argv[3]) : 0);
}
