function [text, diagnostics] = invoke(request, input)
% ProcessBuilder passes argument vectors without a shell (including on Windows).
if nargin < 2, input = ''; end
root = tempname;
[ok, message] = mkdir(root);
assert(ok && isempty(message), 'pelagic:PG0001', 'Cannot reserve private scratch');
cleanup = onCleanup(@() rmdir(root, 's')); %#ok<NASGU>
infile = fullfile(root, 'stdin.json');
outfile = fullfile(root, 'stdout.txt');
errfile = fullfile(root, 'stderr.txt');
fid = fopen(infile, 'w');
assert(fid >= 0, 'pelagic:PG0001', 'Cannot open stdin scratch');
count = fwrite(fid, unicode2native(input, 'UTF-8'), 'uint8');
closed = fclose(fid);
assert(count == numel(unicode2native(input, 'UTF-8')) && closed == 0, 'pelagic:PG0001', 'Cannot write stdin');
cli = getenv('PELAGIC_CLI');
if isempty(cli)
    base = fileparts(fileparts(fileparts(mfilename('fullpath'))));
    if ispc, platform = 'windows-x64'; name = 'pelagic.exe';
    elseif ismac
        if ~isempty(strfind(computer, 'A64')) || ~isempty(strfind(computer, 'aarch64')), platform = 'macos-arm64';
        else, platform = 'macos-x64'; end
        name = 'pelagic';
    else
        if ~isempty(strfind(computer, 'aarch64')) || ~isempty(strfind(computer, 'arm64')), platform = 'linux-arm64';
        else, platform = 'linux-x64'; end
        name = 'pelagic';
    end
    cli = fullfile(base, 'bin', platform, name);
    if ~isfile(cli), cli = name;
    elseif ~ispc
        % Toolbox archives discard Unix execute bits; repair only our bundled binary.
        file = javaObject('java.io.File', cli);
        assert(file.canExecute() || file.setExecutable(true, true), 'pelagic:PG0001', ...
            'Cannot make bundled CLI executable; set PELAGIC_CLI to an installed binary');
    end
end
args = [{cli, '--diagnostics', 'json'}, request];
command = javaArray('java.lang.String', numel(args));
for k = 1:numel(args), command(k) = javaObject('java.lang.String', char(args{k})); end
builder = javaObject('java.lang.ProcessBuilder', command);
builder.redirectInput(javaObject('java.io.File', infile));
builder.redirectOutput(javaObject('java.io.File', outfile));
builder.redirectError(javaObject('java.io.File', errfile));
try
    process = builder.start();
catch exception
    error('pelagic:PG0001', 'Unable to start CLI: %s', exception.message);
end
processCleanup = onCleanup(@() process.destroy()); %#ok<NASGU>
status = process.waitFor();
text = fileread(outfile);
diagnostics = {};
lines = regexp(strtrim(fileread(errfile)), '\r?\n', 'split');
for k = 1:numel(lines)
    if isempty(lines{k}), continue; end
    try
        report = jsondecode(lines{k});
    catch
        error('pelagic:PG0001', 'CLI returned non-protocol stderr: %s', lines{k});
    end
    for j = 1:numel(report.diagnostics)
        diagnostics{end + 1} = report.diagnostics(j); %#ok<AGROW>
    end
end
if status ~= 0
    if isempty(diagnostics), error('pelagic:PG0001', 'CLI failed with exit code %d', status); end
    errors = find(cellfun(@(d) strcmp(d.severity, 'error'), diagnostics), 1);
    if isempty(errors), errors = 1; end
    payload = jsonencode(struct('diagnostics', {diagnostics}));
    error(['pelagic:' diagnostics{errors}.code], '%s', payload);
end
for k = 1:numel(diagnostics)
    warning(['pelagic:' diagnostics{k}.code], '%s', jsonencode(diagnostics{k}));
end
end
