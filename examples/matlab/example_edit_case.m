% Edit a case as a struct: turn the Gulf of Mexico ray trace into transmission loss.
%   addpath('examples/matlab'); example_edit_case     % results go to the current folder
% The struct follows the JSON format in docs/bellhop/json.md. The edited case is
% saved as results/gulf-tl.json, which Python and the command line can also run.
here = fileparts(mfilename('fullpath'));
c = pelagic.bellhop.load(fullfile(here, '..', 'cases', 'bellhop', 'gulf', 'gulf.env'));
fprintf('original run: %s\n', c.bellhop.run);

c.title = 'Gulf of Mexico: coherent transmission loss';
c.bellhop.run = 'coherent';
c.bellhop.beam = 'geometric_gaussian_cartesian';   % ray runs have no beam family
c.bellhop.angles = struct('from', -20, 'to', 20, 'count', 801);  % a grid, like the .env
c.receivers.depth = linspace(0, 5000, 201);        % or a plain vector
c.receivers.range = struct('from', 0, 'to', 120, 'count', 601, 'unit', 'km');
r = pelagic.bellhop.run(c);

if ~isfolder('results'), mkdir('results'); end
fid = fopen(fullfile('results', 'gulf-tl.json'), 'w');
fwrite(fid, r.provenance.case_json);   % the exact JSON that ran
fclose(fid);

loss = squeeze(r.field.transmission_loss_db(1, :, :));
loss(~isfinite(loss)) = 120;
figure;
imagesc(r.ranges_m / 1e3, r.depths_m, loss, [60 110]);
colormap(flipud(jet)); colorbar; xlabel('Range (km)'); ylabel('Depth (m)'); title(r.title);
print('-dpng', fullfile('results', 'example_edit_case.png'));
