function [status, text, reports] = execute(request, input)
% Run the CLI with JSON diagnostics; return its exit status, stdout and each
% stderr line decoded. ProcessBuilder passes argument vectors without a shell
% (including on Windows).
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
args = [{pelagic.internal.cli(), '--diagnostics', 'json'}, request];
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
reports = {};
lines = regexp(strtrim(fileread(errfile)), '\r?\n', 'split');
for k = 1:numel(lines)
    if isempty(lines{k}), continue; end
    try
        reports{end + 1} = jsondecode(lines{k}); %#ok<AGROW>
    catch
        error('pelagic:PG0001', 'CLI returned non-protocol stderr: %s', lines{k});
    end
end
end
