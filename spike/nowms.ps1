# Print current epoch milliseconds. Used to capture agent-wake time: on waking
# from a waiter-completion notification, run this immediately and subtract the
# message's waiter_exit_ms to measure notification/agent-wake latency.
[DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
