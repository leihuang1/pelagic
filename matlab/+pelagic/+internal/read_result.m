function r = read_result(model, path, c)
% HDF5 storage axes are not MATLAB axes. Restore explicit logical dimensions.
octave = exist('OCTAVE_VERSION', 'builtin') ~= 0;
if octave
    data = builtin('load', '-hdf5', path);
    read = @(name) lookup(data, name);
else
    read = @(name) h5read(path, name);
end
r = struct('model', model);
if strcmp(model, 'bellhop')
    r.frequency_hz = c.frequency_hz;
    r.title = c.title;
    if any(strcmp(c.run.kind, {'coherent', 'semi_coherent', 'incoherent'}))
        f.geometry = c.run.receiver_grid;
        f.source_depths_m = column(read('/field/source_depth_m'));
        ranges = column(read('/field/receiver_range_m'));
        depths = column(read('/field/receiver_depth_m'));
        pressure = complex(single(read('/field/pressure_real')), single(read('/field/pressure_imaginary')));
        s = numel(f.source_depths_m);
        if strcmp(f.geometry, 'irregular')
            count = numel(pressure) / s;
            f.ranges_m = ranges(1:count);
            f.depths_m = depths(1:count);
            f.pressure = reshape(pressure, [count, s]).';
        else
            d = numel(c.positions.receiver_depths_m);
            n = numel(c.positions.receiver_ranges_m);
            f.ranges_m = ranges(1:d:d*n);
            f.depths_m = depths(1:d);
            f.pressure = permute(reshape(pressure, [d, n, s]), [3, 1, 2]);
        end
        f.transmission_loss_db = -20 * log10(abs(f.pressure));
        r.field = f;
        % Convenient top-level plotting aliases, with identical logical axes.
        r.pressure = f.pressure; r.ranges_m = f.ranges_m; r.depths_m = f.depths_m;
        r.source_depths_m = f.source_depths_m;
    elseif strcmp(c.run.kind, 'arrivals')
        r.arrivals = columns(read, '/arrivals/', {'source_depth_m', 'receiver_offset', 'arrival_offset', ...
            'receiver_range_m', 'receiver_depth_m', 'amplitude', 'phase_radians', 'travel_time_s', ...
            'attenuation_time_s', 'source_angle_degrees', 'receiver_angle_degrees', 'top_bounces', 'bottom_bounces'});
    elseif strcmp(c.run.kind, 'rays')
        r.rays = columns(read, '/rays/', {'source_depth_m', 'ray_source_index', 'launch_angle_degrees', ...
            'point_offset', 'range_m', 'depth_m', 'travel_time_s', 'attenuation_time_s', ...
            'amplitude', 'phase_radians', 'top_bounces', 'bottom_bounces', 'termination'});
    else
        r.eigenrays = columns(read, '/eigenrays/', {'source_depth_m', 'receiver_offset', 'eigenray_offset', ...
            'receiver_range_m', 'receiver_depth_m', 'launch_angle_degrees', 'point_offset', ...
            'range_m', 'depth_m', 'travel_time_s', 'attenuation_time_s', 'amplitude', 'phase_radians', ...
            'top_bounces', 'bottom_bounces'});
    end
else
    hz = column(read('/frequency_hz'));
    r.frequencies = cell(1, numel(hz));
    for i = 1:numel(hz)
        prefix = sprintf('/frequencies/%d/', i - 1);
        definition = item(c.frequencies, i);
        profiles = cell(1, numel(definition.profiles));
        for j = 1:numel(profiles)
            if j == 1, modepath = [prefix 'modes/'];
            else, modepath = [prefix sprintf('profiles/%d/modes/', j - 1)]; end
            m.depths_m = column(read([modepath 'sample_depth_m']));
            m.wavenumbers = complex(column(read([modepath 'horizontal_wavenumber_real'])), column(read([modepath 'horizontal_wavenumber_imaginary'])));
            m.eigenfunctions = reshape(complex(read([modepath 'eigenfunction_real']), read([modepath 'eigenfunction_imaginary'])), [numel(m.depths_m), numel(m.wavenumbers)]);
            m.phase_speed_mps = column(read([modepath 'phase_speed_mps']));
            m.group_speed_mps = column(read([modepath 'group_speed_mps']));
            m.attenuation_nepers_per_m = column(read([modepath 'attenuation_nepers_per_m']));
            profiles{j} = struct('range_m', item(definition.profile_ranges_m, j), 'modes', m);
        end
        f.geometry = 'rectilinear';
        f.source_depths_m = column(read([prefix 'field/source_depth_m']));
        f.depths_m = column(read([prefix 'field/receiver_depth_m']));
        f.ranges_m = column(read([prefix 'field/receiver_range_m']));
        f.receiver_offsets_m = column(read([prefix 'field/receiver_offset_m']));
        pressure = complex(single(read([prefix 'field/pressure_real'])), single(read([prefix 'field/pressure_imaginary'])));
        f.pressure = permute(reshape(pressure, [numel(f.ranges_m), numel(f.depths_m), numel(f.source_depths_m)]), [3, 2, 1]);
        f.transmission_loss_db = -20 * log10(abs(f.pressure));
        r.frequencies{i} = struct('frequency_hz', hz(i), 'profiles', {profiles}, 'field', f);
    end
end
end

function result = columns(read, prefix, names)
result = struct();
for k = 1:numel(names), result.(names{k}) = column(read([prefix names{k}])); end
end
function value = column(value), value = value(:); end
function value = item(values, index)
if iscell(values), value = values{index}; else, value = values(index); end
end
function value = lookup(value, path)
parts = regexp(path, '/', 'split');
for k = 1:numel(parts)
    name = parts{k};
    if isempty(name), continue; end
    if ~isfield(value, name)
        % Octave makes digit-leading HDF5 group names valid identifiers.
        if isfield(value, ['_' name]), name = ['_' name];
        elseif isfield(value, ['x' name]), name = ['x' name];
        else, error('pelagic:PG0402', 'Missing HDF5 group %s', path); end
    end
    value = value.(name);
end
end
