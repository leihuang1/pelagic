function [text, diagnostics] = invoke(request, input)
% Run the CLI; a failure raises its first error, and warnings are re-issued.
if nargin < 2, input = ''; end
[status, text, reports] = pelagic.internal.execute(request, input);
diagnostics = {};
for k = 1:numel(reports)
    for j = 1:numel(reports{k}.diagnostics)
        diagnostics{end + 1} = reports{k}.diagnostics(j); %#ok<AGROW>
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
