function r = run(c, varargin)
% Solve ordered frequency/profile blocks, retaining duplicates and mode counts.
% run(c, 'Threads', n) solves on up to n threads (default: all CPUs);
% results are identical for every thread count.
r = pelagic.internal.run('kraken', c, varargin{:});
end
