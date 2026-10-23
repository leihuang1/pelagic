function [c, diagnostics] = load(path)
% Export legacy ENV+resources or JSON to a portable struct, without solving.
pelagic.version();
[text, diagnostics] = pelagic.internal.invoke({'bellhop', 'export', char(path)});
c = pelagic.internal.decode(text);
end
