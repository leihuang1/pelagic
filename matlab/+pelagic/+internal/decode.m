function value = decode(text)
% Decode JSON: arrays of numbers become numeric row vectors (including one-element
% arrays); other non-empty arrays become cells; empty arrays become {} so they can
% be appended to. encode() writes them back as the same JSON shapes.
% This is a transport codec, not a second acoustic schema or validator.
[value, position] = parse(text, 1);
assert(isempty(strtrim(text(position:end))), 'pelagic:PG0103', 'Trailing JSON data');
end

function [value, position] = parse(text, position)
position = space(text, position);
switch text(position)
    case '{'
        value = struct();
        position = space(text, position + 1);
        while text(position) ~= '}'
            [name, position] = string_value(text, position);
            position = space(text, position);
            assert(text(position) == ':', 'pelagic:PG0103', 'Expected colon');
            [item, position] = parse(text, position + 1);
            value.(name) = item;
            position = space(text, position);
            if text(position) ~= ',', break; end
            position = space(text, position + 1);
        end
        assert(text(position) == '}', 'pelagic:PG0103', 'Expected object end');
        position = position + 1;
    case '['
        value = {};
        nested = false;
        position = space(text, position + 1);
        while text(position) ~= ']'
            % A one-element row such as [1500] decodes to a scalar; keep its parent a cell.
            nested = nested || text(position) == '[';
            [item, position] = parse(text, position);
            value{end + 1} = item; %#ok<AGROW>
            position = space(text, position);
            if text(position) ~= ',', break; end
            position = space(text, position + 1);
        end
        assert(text(position) == ']', 'pelagic:PG0103', 'Expected array end');
        position = position + 1;
        if ~isempty(value) && ~nested && all(cellfun(@(x) isnumeric(x) && isscalar(x), value))
            value = [value{:}];
        end
    case '"'
        [value, position] = string_value(text, position);
    otherwise
        start = position;
        while position <= length(text) && ~isspace(text(position)) && ~any(text(position) == ',]}')
            position = position + 1;
        end
        token = text(start:position - 1);
        if strcmp(token, 'null'), value = NaN;
        % Octave's jsondecode fast number parser can round by one ULP.
        elseif any(token(1) == '-0123456789'), value = str2double(token);
        else, value = jsondecode(token); end
end
end

function [value, position] = string_value(text, position)
start = position;
position = position + 1;
while text(position) ~= '"'
    if text(position) == '\', position = position + 1; end
    position = position + 1;
end
value = jsondecode(text(start:position));
position = position + 1;
end

function position = space(text, position)
while position <= length(text) && isspace(text(position))
    position = position + 1;
end
end
