// The client patcher's window. rpatchur loads this page and calls the
// patchingStatus* functions below as it works; the buttons call back into
// it with external.invoke. Plain ES5: it runs in Windows' old IE engine.
(function () {
  var $ = function (id) { return document.getElementById(id); };
  // In the patcher, external.invoke is a native method ("unknown" or
  // "function" to typeof). In a normal browser it's missing.
  var inPatcher = !!window.external && typeof window.external.invoke !== 'undefined';

  function invoke(cmd) {
    if (inPatcher) {
      window.external.invoke(cmd);
    }
  }

  function status(text, isError) {
    $('status').innerText = text;
    $('status').className = isError ? 'error' : '';
  }

  function detail(text) {
    $('detail').innerText = text || ' ';
  }

  function progress(done, total) {
    $('progress').style.width = (total > 0 ? Math.round(100 * done / total) : 100) + '%';
  }

  function size(bytes) {
    var units = ['B', 'KB', 'MB', 'GB'];
    var i = 0;
    while (bytes >= 1024 && i < units.length - 1) {
      bytes /= 1024;
      i++;
    }
    return (i ? bytes.toFixed(1) : bytes) + ' ' + units[i];
  }

  function setPlay(on) {
    $('play').disabled = !on;
    $('retry').style.display = 'none';
  }

  window.patchingStatusReady = function () {
    status('Up to date. Ready to play.');
    detail('');
    progress(1, 1);
    setPlay(true);
  };

  window.patchingStatusError = function (msg) {
    status('Update failed: ' + msg, true);
    detail('Check that this PC can reach the server, then try again.');
    setPlay(false);
    $('retry').style.display = '';
  };

  window.patchingStatusDownloading = function (done, total, bytesPerSec) {
    status('Downloading update ' + Math.min(done + 1, total) + ' of ' + total + '...');
    detail(bytesPerSec > 0 ? size(bytesPerSec) + '/s' : '');
    progress(done, total);
    setPlay(false);
  };

  window.patchingStatusInstalling = function (done, total) {
    status('Installing update ' + Math.min(done + 1, total) + ' of ' + total + '...');
    progress(done, total);
    setPlay(false);
  };

  window.patchingStatusPatchApplied = function (name) {
    detail('Installed ' + name);
  };

  window.notificationInProgress = function () {
    detail('An update is already running.');
  };

  $('play').onclick = function () { invoke('play'); };
  $('exit').onclick = function () { invoke('exit'); };
  $('retry').onclick = function () {
    status('Checking for updates...');
    detail('');
    $('retry').style.display = 'none';
    invoke('start_update');
  };

  // Links open in the player's browser, not in the patcher's window.
  var links = document.getElementsByTagName('a');
  for (var i = 0; i < links.length; i++) {
    if (links[i].getAttribute('data-open') === null) {
      continue;
    }
    links[i].onclick = function () {
      if (!inPatcher) {
        return true;
      }
      invoke(JSON.stringify({ 'function': 'open_url', parameters: { url: this.href } }));
      return false;
    };
  }

  if (inPatcher) {
    invoke('start_update');
  } else {
    status('This page is the patcher\'s window. Download the patcher from the How to connect page.');
  }
})();
