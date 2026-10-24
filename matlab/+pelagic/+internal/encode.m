function text = encode(value)
% Encode a case struct as JSON with one writer for MATLAB and Octave.
% Numbers use round-trip %.17g; Octave's jsonencode can flush tiny nonzero values
% to zero. Every case-JSON list also accepts a single number, so a MATLAB scalar
% may stay a JSON number; matrices and cells of rows become arrays of arrays.
try
    text = encode_value(value, false);
catch exception
    diagnostic = struct('code', 'PG0103', 'severity', 'error', 'field', 'json', ...
        'message', exception.message, 'source', NaN);
    error('pelagic:PG0103', '%s', jsonencode(struct('diagnostics', {{diagnostic}})));
end
end

function text = encode_value(value, as_array)
if isstruct(value) && isscalar(value)
    names = fieldnames(value); parts = cell(size(names));
    for k = 1:numel(names)
        parts{k} = [jsonencode(names{k}), ':', encode_value(value.(names{k}), false)];
    end
    text = ['{', strjoin(parts, ','), '}'];
elseif isstruct(value) || iscell(value)
    % A cell of plain numbers is the {20, 30} spelling of a flat list: every table
    % row has at least two values. Otherwise a number in a cell is one table row,
    % such as a sound-speed row [depth, cp], and stays an array.
    flat = iscell(value) && all(cellfun(@(x) isnumeric(x) && isscalar(x), value(:)));
    parts = cell(1, numel(value));
    for k = 1:numel(value)
        if iscell(value), item = value{k}; else, item = value(k); end
        parts{k} = encode_value(item, ~flat && isnumeric(item));
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
