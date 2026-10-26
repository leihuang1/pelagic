function r = run(c, varargin)
% Validate and solve a JSON-compatible struct through the matching CLI.
% run(c, 'Threads', n) solves on up to n threads (default: all CPUs);
% results are identical for every thread count.
r = pelagic.internal.run('bellhop', c, varargin{:});
end
