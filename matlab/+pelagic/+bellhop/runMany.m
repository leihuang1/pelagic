function [results, failures] = runMany(cases, varargin)
% Solve a cell array of case structs or file paths with one CLI call.
% [results, failures] = runMany(cases, 'Workers', w, 'Threads', t, 'Output', dir)
% results{k} is what run would return for cases{k} (a path is loaded first),
% or [] if that case failed. failures lists index, code, message and
% diagnostics for each failed case. With one output, a failure is thrown once
% every case has finished. 'Output' keeps each case's files in dir/c00001/, ...
% Results are identical for every split into workers and threads.
[results, failures] = pelagic.internal.run_many('bellhop', nargout < 2, cases, varargin{:});
end
