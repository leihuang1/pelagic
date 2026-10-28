"""Bounded independent-case concurrency; never parallelizes a KRAKEN frequency sequence."""
from collections.abc import Sized
from concurrent.futures import ThreadPoolExecutor, wait, FIRST_COMPLETED
from contextlib import contextmanager
import signal
from threading import Lock, current_thread, main_thread
from . import _core
from .errors import CancelledError, PelagicError
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


def run_many(cases, workers=None, threads=None, return_exceptions=False):
    """Yield in input order, retaining at most workers running/ready results.

    By default there is one worker per case, up to one per available CPU (or
    CPUs // threads when threads is given), and each solve uses the CPUs left
    per worker. Results are identical for every split.

    A failed case raises its error and cancels the remaining work. With
    return_exceptions=True a case's PelagicError, other than CancelledError, is
    yielded in that case's place instead, and the sweep continues. Interruption
    always cancels and joins remaining work. Use a with-block when stopping
    iteration early, so abandoned work is cancelled immediately.
    """
    for name, value in (("workers", workers), ("threads", threads)):
        if value is not None and (isinstance(value, bool) or not isinstance(value, int) or value < 1):
            raise ValueError(f"{name} must be a positive integer")
    if not isinstance(return_exceptions, bool):
        raise ValueError("return_exceptions must be True or False")
    available = _core.available_threads()
    if workers is None:
        workers = max(1, available // (threads or 1))
        if isinstance(cases, Sized):
            workers = max(1, min(workers, len(cases)))
    if threads is None:
        threads = max(1, available // workers)
    return _Sweep(_iterate(cases, workers, threads, return_exceptions))


def _iterate(cases, workers, threads, return_exceptions):
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

    def isolated(error):
        # A case's own failure, yielded in its place; cancellation still ends the sweep.
        return return_exceptions and isinstance(error, PelagicError) and not isinstance(error, CancelledError)

    def completed(future, index):
        # Cancels siblings promptly while the consumer is busy; the main loop records
        # the same failure itself because wait() can return before callbacks run.
        if not future.cancelled() and future.exception() is not None and not isolated(future.exception()):
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
                    future = executor.submit(case._run_native, token, threads)
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
                yield native if isinstance(native, PelagicError) else from_native(native)
                del native
                continue
            if not active:
                return
            done, _ = wait(active, timeout=0.05, return_when=FIRST_COMPLETED)
            if failures:
                raise failures[0]
            for future in done:
                index = active.pop(future)
                error = future.exception()
                if error is None:
                    ready[index] = future.result()
                elif isolated(error):
                    error.case_index = index
                    ready[index] = error
                else:
                    fail(error, index)
                    raise failures[0]
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
