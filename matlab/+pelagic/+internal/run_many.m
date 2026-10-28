function [results, failures] = run_many(model, raise, cases, varargin)
% Solve independent cases with one `run-many` call. Each case is written as
% cNNNNN.json, by its index; a path is loaded first, as for run(load(path)).
% A failed case leaves [] in results and an entry in failures. With raise,
% the first failure is thrown once every case has finished.
if ~iscell(cases)
    error('pelagic:PG0002', 'Expected a cell array of case structs or file paths');
end
for k = 1:numel(cases)
    item = cases{k};
    if ~((isstruct(item) && isscalar(item)) || is_text(item))
        error('pelagic:PG0002', 'Case %d is neither a case struct nor a file path', k);
    end
end
[workers, threads, output] = options(varargin);
results = cell(size(cases));
failures = struct('index', {}, 'code', {}, 'message', {}, 'diagnostics', {});
if isempty(cases), return; end
info = pelagic.version();
assert(info.run_many_status_version == 1, 'pelagic:PG0202', 'Unsupported CLI protocol');
root = tempname;
[ok, message] = mkdir(root);
assert(ok && isempty(message), 'pelagic:PG0001', 'Cannot reserve run-many scratch');
cleanup = onCleanup(@() rmdir(root, 's')); %#ok<NASGU>
inputs = fullfile(root, 'cases');
[ok, message] = mkdir(inputs);
assert(ok && isempty(message), 'pelagic:PG0001', 'Cannot reserve run-many scratch');
if isempty(output), output = fullfile(root, 'results'); end

pattern = sprintf('c%%0%dd', max(5, numel(sprintf('%d', numel(cases)))));
names = cell(size(cases)); structs = names; texts = names;
pending = false(size(cases));
for k = 1:numel(cases)
    names{k} = sprintf(pattern, k);
    try
        c = cases{k};
        if ~isstruct(c)
            if strcmp(model, 'bellhop'), c = pelagic.bellhop.load(char(c));
            else, c = pelagic.kraken.load(char(c)); end
        end
        text = pelagic.internal.encode(c);
        write_text(fullfile(inputs, [names{k} '.json']), text);
        structs{k} = c; texts{k} = text; pending(k) = true;
    catch exception
        failures(end + 1) = failure(k, cells(pelagic.diagnostics(exception))); %#ok<AGROW>
    end
end

if any(pending(:))
    % Only records appended by this call count, if Output already has some.
    status_file = fullfile(output, 'status.ndjson');
    listing = dir(status_file);
    offset = 0;
    if numel(listing) == 1, offset = listing.bytes; end
    request = [{model, 'run-many', inputs, '--output', output, '--format', 'hdf5'}, workers, threads];
    [status, ~, reports] = pelagic.internal.execute(request);
    % Reports naming a case are in the status file; the others concern the whole call.
    batch = {};
    for k = 1:numel(reports)
        if isequal(fieldnames(reports{k}), {'diagnostics'})
            batch = [batch, cells(reports{k}.diagnostics)]; %#ok<AGROW>
        end
    end
    errors = find(cellfun(@(d) strcmp(d.severity, 'error'), batch), 1);
    if status ~= 0 && ~isempty(errors)
        payload = jsonencode(struct('diagnostics', {batch}));
        error(['pelagic:' batch{errors}.code], '%s', payload);
    end
    for k = 1:numel(batch)
        warning(['pelagic:' batch{k}.code], '%s', jsonencode(batch{k}));
    end
    records = read_records(status_file, offset, names);
    for k = find(pending(:)).'
        record = records{k};
        if isempty(record)
            failures(end + 1) = failure(k, cells(struct('code', 'PG0001', 'severity', 'error', ...
                'field', 'cli', 'message', sprintf('The CLI exited with code %d without reporting this case', status), ...
                'source', NaN))); %#ok<AGROW>
            continue
        end
        diagnostics = cells(record.diagnostics);
        if ~strcmp(record.outcome, 'ok')
            failures(end + 1) = failure(k, diagnostics); %#ok<AGROW>
            continue
        end
        try
            r = pelagic.internal.read_result(model, fullfile(output, names{k}, [names{k} '.h5']), structs{k});
        catch exception
            failures(end + 1) = failure(k, cells(pelagic.diagnostics(exception))); %#ok<AGROW>
            continue
        end
        r.provenance = info;
        r.provenance.case_json = texts{k};
        r.diagnostics = diagnostics;
        results{k} = r;
        for j = 1:numel(diagnostics)
            warning(['pelagic:' diagnostics{j}.code], 'case %d: %s', k, jsonencode(diagnostics{j}));
        end
    end
end

if ~isempty(failures)
    [~, order] = sort([failures.index]);
    failures = failures(order);
end
if raise && ~isempty(failures)
    first = failures(1);
    payload = jsonencode(struct('index', first.index, 'failure_count', numel(failures), ...
        'diagnostics', {first.diagnostics}));
    error(['pelagic:' first.code], '%s', payload);
end
end

function [workers, threads, output] = options(args)
workers = {}; threads = {}; output = '';
if mod(numel(args), 2) ~= 0
    error('pelagic:PG0002', 'Expected name/value pairs: ''Workers'', ''Threads'' or ''Output''');
end
for k = 1:2:numel(args)
    name = args{k}; value = args{k + 1};
    if ~is_text(name), error('pelagic:PG0002', 'Expected an option name'); end
    switch lower(char(name))
        case 'workers', workers = {'--workers', count(value, 'Workers')};
        case 'threads', threads = {'--threads', count(value, 'Threads')};
        case 'output'
            if ~is_text(value), error('pelagic:PG0002', 'Output must be a folder name'); end
            output = char(value);
        otherwise
            error('pelagic:PG0002', 'Unknown option %s; expected Workers, Threads or Output', char(name));
    end
end
end

function text = count(n, name)
if ~(isnumeric(n) && isscalar(n) && isreal(n) && isfinite(n) && n >= 1 && n == fix(n))
    error('pelagic:PG0002', '%s must be a positive integer', name);
end
text = sprintf('%d', n);
end

function yes = is_text(value)
% A non-empty character row or string scalar.
yes = (ischar(value) && size(value, 1) == 1 && ~isempty(value)) ...
    || (isstring(value) && isscalar(value) && strlength(value) > 0);
end

function write_text(path, text)
bytes = unicode2native(text, 'UTF-8');
fid = fopen(path, 'w');
assert(fid >= 0, 'pelagic:PG0001', 'Cannot open %s', path);
count = fwrite(fid, bytes, 'uint8');
closed = fclose(fid);
assert(count == numel(bytes) && closed == 0, 'pelagic:PG0001', 'Cannot write %s', path);
end

function records = read_records(path, offset, names)
% The last complete record for each case, from the bytes after offset.
records = cell(size(names));
fid = fopen(path, 'r');
if fid < 0, return; end
closer = onCleanup(@() fclose(fid));
fseek(fid, offset, 'bof');
bytes = fread(fid, Inf, '*uint8');
lines = regexp(native2unicode(bytes(:).', 'UTF-8'), '\r?\n', 'split');
for k = 1:numel(lines)
    try
        record = jsondecode(lines{k});
        index = find(strcmp(record.stem, names), 1);
    catch
        continue
    end
    if ~isempty(index), records{index} = record; end
end
end

function entry = failure(index, diagnostics)
% The first error names the failure.
code = 'PG0001'; message = 'The case failed without a diagnostic';
if ~isempty(diagnostics)
    first = find(cellfun(@(d) strcmp(d.severity, 'error'), diagnostics), 1);
    if isempty(first), first = 1; end
    code = diagnostics{first}.code; message = diagnostics{first}.message;
end
entry = struct('index', index, 'code', code, 'message', message, 'diagnostics', {diagnostics});
end

function items = cells(values)
% Diagnostics as a row cell of structs, as run returns them.
if iscell(values), items = reshape(values, 1, []);
elseif isempty(values), items = {};
else, items = reshape(num2cell(values), 1, []); end
end
