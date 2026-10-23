% Edit a case as a struct: turn the Gulf of Mexico ray trace into transmission loss.
%   addpath('examples/matlab'); example_edit_case     % results go to the current folder
% The struct follows the JSON format in docs/bellhop/json.md. The edited case is
% saved as results/gulf-tl.json, which Python and the command line can also run.
here = fileparts(mfilename('fullpath'));
c = pelagic.bellhop.load(fullfile(here, '..', 'cases', 'bellhop', 'gulf', 'gulf.env'));
fprintf('original run: %s with %d rays\n', c.run.kind, numel(c.trace.launch_angles_degrees));

c.title = 'Gulf of Mexico: coherent transmission loss';
c.run.kind = 'coherent';
c.run.beam_family = 'geometric_gaussian_cartesian';
c.trace.launch_angles_degrees = linspace(-20, 20, 801);
c.positions.receiver_depths_m = linspace(0, 5000, 201);
c.positions.receiver_ranges_m = linspace(0, 120e3, 601);
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
