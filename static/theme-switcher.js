(function () {
  var THEMES = ['golden-hour', 'original'];
  var STORAGE_KEY = 'gpx-theme';

  function current() {
    return localStorage.getItem(STORAGE_KEY) || 'golden-hour';
  }

  function apply(theme) {
    if (theme === 'original') {
      document.documentElement.removeAttribute('data-theme');
    } else {
      document.documentElement.dataset.theme = theme;
    }
    localStorage.setItem(STORAGE_KEY, theme);
  }

  apply(current());

  document.addEventListener('DOMContentLoaded', function () {
    var nav = document.querySelector('.site-nav');
    if (!nav) return;

    var btn = document.createElement('button');
    btn.className = 'theme-toggle';
    btn.setAttribute('aria-label', 'Switch theme');
    btn.title = 'Switch theme';
    btn.innerHTML = '<span class="swatch"></span>';

    btn.addEventListener('click', function () {
      var now = current();
      var next = THEMES[(THEMES.indexOf(now) + 1) % THEMES.length];
      apply(next);
    });

    nav.appendChild(btn);
  });
})();
