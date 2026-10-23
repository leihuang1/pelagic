function r = run(model, c)
info = pelagic.version();
root = tempname;
[ok, message] = mkdir(root);
assert(ok && isempty(message), 'pelagic:PG0001', 'Cannot reserve result scratch');
cleanup = onCleanup(@() rmdir(root, 's')); %#ok<NASGU>
text = pelagic.internal.encode(c);
[~, diagnostics] = pelagic.internal.invoke({model, 'run', '-', '--format', 'hdf5', '--output', fullfile(root, 'result')}, text);
r = pelagic.internal.read_result(model, fullfile(root, 'result', 'stdin.h5'), c);
r.provenance = info;
r.provenance.case_json = text;
r.diagnostics = diagnostics;
end
