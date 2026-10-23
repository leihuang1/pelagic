function text = encode(value)
% Encode a case struct as JSON with one writer for MATLAB and Octave.
% Numbers use round-trip %.17g; Octave's jsonencode can flush tiny nonzero values
% to zero. Schema array fields and numbers nested in cells are always JSON arrays.
try
    text = encode_value(value, false, pelagic.internal.array_fields());
catch exception
    diagnostic = struct('code', 'PG0103', 'severity', 'error', 'field', 'json', ...
        'message', exception.message, 'source', NaN);
    error('pelagic:PG0103', '%s', jsonencode(struct('diagnostics', {{diagnostic}})));
end
end

function text = encode_value(value, as_array, arrays)
if isstruct(value) && isscalar(value)
    names = fieldnames(value); parts = cell(size(names));
    for k = 1:numel(names)
        item = value.(names{k});
        parts{k} = [jsonencode(names{k}), ':', ...
            encode_value(item, any(strcmp(names{k}, arrays)), arrays)];
    end
    text = ['{', strjoin(parts, ','), '}'];
elseif isstruct(value) || iscell(value)
    % A cell of plain numbers under an array field is the older {20, 30} spelling of a
    % flat list. Otherwise decoding has already turned all-number arrays into vectors,
    % so a number left in a cell was itself an array (one row of a sound-speed matrix).
    flat = as_array && iscell(value) && all(cellfun(@(x) isnumeric(x) && isscalar(x), value(:)));
    parts = cell(1, numel(value));
    for k = 1:numel(value)
        if iscell(value), item = value{k}; else, item = value(k); end
        parts{k} = encode_value(item, ~flat && isnumeric(item), arrays);
    end
    text = ['[', strjoin(parts, ','), ']'];
elseif isnumeric(value)
    assert(isreal(value) && ndims(value) <= 2, 'JSON numbers must be real scalars, vectors or matrices');
    if isempty(value), text = '[]';
    elseif isscalar(value) && ~as_array, text = number(value);
    elseif isvector(value), text = ['[', numbers(value(:).'), ']'];
    else
        parts = cell(1, size(value, 1));
        for k = 1:size(value, 1), parts{k} = ['[', numbers(value(k, :)), ']']; end
        text = ['[', strjoin(parts, ','), ']'];
    end
else
    text = jsonencode(value);
end
end

function text = numbers(values)
parts = arrayfun(@number, values, 'UniformOutput', false);
text = strjoin(parts, ',');
end

function text = number(value)
if ~isfinite(value), text = 'null';
elseif isinteger(value), text = sprintf('%d', value);
else, text = sprintf('%.17g', value); end
end
