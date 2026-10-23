function items = diagnostics(exception)
% Recover Rust reports or a structured adapter failure without hiding the original error.
try
    report = jsondecode(exception.message);
    items = report.diagnostics;
catch
    if ~strncmp(exception.identifier, 'pelagic:', 8), rethrow(exception); end
    code = exception.identifier(9:end);
    field = 'cli';
    if strcmp(code, 'PG0103'), field = 'json';
    elseif strcmp(code, 'PG0202'), field = 'cli.version'; end
    items = struct('code', code, 'severity', 'error', 'field', field, ...
        'message', exception.message, 'source', NaN);
end
end
