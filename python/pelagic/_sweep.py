"""Bounded independent-case concurrency; never parallelizes a KRAKEN frequency sequence."""
from concurrent.futures import ThreadPoolExecutor, wait, FIRST_COMPLETED
from contextlib import contextmanager
import signal
from threading import Lock, current_thread, main_thread
from . import _core
from .results import from_native


@contextmanager
def _defer_interrupt():
    handler = signal.getsignal(signal.SIGINT)
    if current_thread() is not main_thread() or not callable(handler):
        yield
        return
    pending = []
    signal.signal(signal.SIGINT, lambda number, frame: pending.append((number, frame)))
    try:
        yield
    finally:
        signal.signal(signal.SIGINT, handler)
        if pending:
            handler(*pending[0])


class _Sweep:
    def __init__(self, iterator):
        self._iterator = iterator

    def __iter__(self):
        return self

    def __next__(self):
        return next(self._iterator)

    def close(self):
        self._iterator.close()

    def __enter__(self):
        return self

    def __exit__(self, *exc):
        self.close()


def run_many(cases, workers=1):
    """Yield in input order, retaining at most workers running/ready results.

    On failure or interruption cancel and join remaining work. Use a with-block
    when stopping iteration early, so abandoned work is cancelled immediately.
    """
    if isinstance(workers, bool) or not isinstance(workers, int) or workers < 1:
        raise ValueError("workers must be a positive integer")
    return _Sweep(_iterate(cases, workers))


def _iterate(cases, workers):
    token = _core.Cancellation()
    executor = ThreadPoolExecutor(max_workers=workers, thread_name_prefix="pelagic")
    source = iter(cases)
    active, ready = {}, {}
    submitted = expected = 0
    exhausted = False
    failures = []
    lock = Lock()

    def fail(error, index):
        with lock:
            if not failures:
                error.case_index = index
                failures.append(error)
                token.cancel()

    def completed(future, index):
        # Cancels siblings promptly while the consumer is busy; the main loop records
        # the same failure itself because wait() can return before callbacks run.
        if not future.cancelled() and future.exception() is not None:
            fail(future.exception(), index)

    try:
        while True:
            if failures:
                raise failures[0]
            while not exhausted and len(active) + len(ready) < workers:
                try:
                    case = next(source)
                except StopIteration:
                    exhausted = True
                    break
                # ThreadPoolExecutor registers a new thread AFTER starting it; defer
                # SIGINT until both its registration and our future bookkeeping finish.
                with _defer_interrupt():
                    future = executor.submit(case._run_native, token)
                    active[future] = submitted
                    future.add_done_callback(lambda f, i=submitted: completed(f, i))
                    submitted += 1
                if failures:
                    raise failures[0]
            if failures:
                raise failures[0]
            if expected in ready:
                native = ready.pop(expected)
                expected += 1
                yield from_native(native)
                del native
                continue
            if not active:
                return
            done, _ = wait(active, timeout=0.05, return_when=FIRST_COMPLETED)
            if failures:
                raise failures[0]
            for future in done:
                index = active.pop(future)
                if future.exception() is not None:
                    fail(future.exception(), index)
                    raise failures[0]
                ready[index] = future.result()
    finally:
        token.cancel()
        for future in active:
            future.cancel()
        interrupted = False
        # Drain native work before joining: an interrupted Thread.join can mark a live
        # thread stopped on some CPython versions. Futures track actual solve completion.
        pending = {future for future in active if not future.done()}
        while pending:
            try:
                wait(pending, timeout=0.05)
                pending = {future for future in pending if not future.done()}
            except KeyboardInterrupt:
                interrupted = True
        while True:
            try:
                with _defer_interrupt():
                    executor.shutdown(wait=True, cancel_futures=True)
                break
            except KeyboardInterrupt:
                interrupted = True
        if interrupted:
            raise KeyboardInterrupt
