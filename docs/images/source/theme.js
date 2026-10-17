// ?theme=dark switches the page to its dark colours; render.sh shoots each page once per theme.
if (new URLSearchParams(location.search).get('theme') === 'dark') document.documentElement.dataset.theme = 'dark';
