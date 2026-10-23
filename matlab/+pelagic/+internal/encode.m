function text = encode(value)
% Octave's JSON writer can flush tiny nonzero coefficients to zero.
try
    if exist('OCTAVE_VERSION', 'builtin'), text = octave_value(value);
    else, text = jsonencode(value); end
catch exception
    diagnostic = struct('code', 'PG0103', 'severity', 'error', 'field', 'json', ...
        'message', exception.message, 'source', NaN);
    error('pelagic:PG0103', '%s', jsonencode(struct('diagnostics', {{diagnostic}})));
end
end

function text = octave_value(value)
if isstruct(value) && isscalar(value)
    names = fieldnames(value); parts = cell(size(names));
    for k = 1:numel(names)
        parts{k} = [jsonencode(names{k}), ':', octave_value(value.(names{k}))];
    end
    text = ['{', strjoin(parts, ','), '}'];
elseif isstruct(value) || iscell(value)
    parts = cell(1, numel(value));
    for k = 1:numel(value)
        if iscell(value), item = value{k}; else, item = value(k); end
        parts{k} = octave_value(item);
    end
    text = ['[', strjoin(parts, ','), ']'];
elseif isnumeric(value)
    assert(isreal(value) && ndims(value) <= 2, 'JSON numbers must be real scalars, vectors or matrices');
    if isempty(value), text = '[]';
    elseif isscalar(value)
        if ~isfinite(value), text = 'null';
        elseif isinteger(value), text = jsonencode(value);
        else, text = sprintf('%.17g', value); end
    elseif isvector(value)
        parts = arrayfun(@octave_value, value(:).', 'UniformOutput', false);
        text = ['[', strjoin(parts, ','), ']'];
    else
        parts = cell(1, size(value, 1));
        for k = 1:size(value, 1), parts{k} = octave_value(value(k, :)); end
        text = ['[', strjoin(parts, ','), ']'];
    end
else
    text = jsonencode(value);
end
end
