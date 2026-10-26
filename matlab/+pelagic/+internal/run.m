function r = run(model, c, varargin)
threads = {};
if ~isempty(varargin)
    if numel(varargin) ~= 2 || ~ischar(varargin{1}) || ~strcmpi(varargin{1}, 'Threads')
        error('pelagic:PG0002', 'Expected run(case) or run(case, ''Threads'', n)');
    end
    n = varargin{2};
    if ~(isnumeric(n) && isscalar(n) && isreal(n) && isfinite(n) && n >= 1 && n == fix(n))
        error('pelagic:PG0002', 'Threads must be a positive integer');
    end
    threads = {'--threads', sprintf('%d', n)};
end
info = pelagic.version();
root = tempname;
[ok, message] = mkdir(root);
assert(ok && isempty(message), 'pelagic:PG0001', 'Cannot reserve result scratch');
cleanup = onCleanup(@() rmdir(root, 's')); %#ok<NASGU>
text = pelagic.internal.encode(c);
[~, diagnostics] = pelagic.internal.invoke([{model, 'run', '-', '--format', 'hdf5', '--output', fullfile(root, 'result')}, threads], text);
r = pelagic.internal.read_result(model, fullfile(root, 'result', 'stdin.h5'), c);
r.provenance = info;
r.provenance.case_json = text;
r.diagnostics = diagnostics;
end
