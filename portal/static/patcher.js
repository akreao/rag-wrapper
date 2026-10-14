// The client patcher's window. rpatchur loads this page and calls the
// patchingStatus* functions below as it works; the buttons call back into
// it with external.invoke. Plain ES5: it runs in Windows' old IE engine.
(function () {
  var $ = function (id) { return document.getElementById(id); };
  // How long a check may go without any word from the patcher before the
  // page offers to try again.
  var answerTimeout = 30000;
  var waiting = null;

  // The patcher's window has no developer tools, so a script error shows
  // on the page instead of leaving it stuck.
  window.onerror = function (msg, url, line) {
    var el = $('detail');
    if (el) {
      el.innerText = 'Page error: ' + msg + ' (line ' + line + ')';
    }
  };

  // Sends a command to the patcher, and says whether it could. Don't test
  // typeof window.external.invoke first: rpatchur's IE host answers a
  // property read of invoke with nothing, so typeof is "undefined" there
  // even though the call works. In a normal browser the call throws.
  function invoke(cmd) {
    try {
      window.external.invoke(cmd);
      return true;
    } catch (e) {
      return false;
    }
  }

  // Any word from the patcher ends the wait for an answer.
  function answered() {
    if (waiting !== null) {
      clearTimeout(waiting);
      waiting = null;
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
    answered();
    status('Up to date. Ready to play.');
    detail('');
    progress(1, 1);
    setPlay(true);
  };

  window.patchingStatusError = function (msg) {
    answered();
    status('Update failed: ' + msg, true);
    detail('Check that this PC can reach the server, then try again.');
    setPlay(false);
    $('retry').style.display = '';
  };

  window.patchingStatusDownloading = function (done, total, bytesPerSec) {
    answered();
    status('Downloading update ' + Math.min(done + 1, total) + ' of ' + total + '...');
    detail(bytesPerSec > 0 ? size(bytesPerSec) + '/s' : '');
    progress(done, total);
    setPlay(false);
  };

  window.patchingStatusInstalling = function (done, total) {
    answered();
    status('Installing update ' + Math.min(done + 1, total) + ' of ' + total + '...');
    progress(done, total);
    setPlay(false);
  };

  window.patchingStatusPatchApplied = function (name) {
    answered();
    detail('Installed ' + name);
  };

  window.notificationInProgress = function () {
    answered();
    detail('An update is already running.');
  };

  // Asks the patcher for any new patches. If it never answers, say so
  // rather than leaving "Checking for updates..." up for good.
  function checkForUpdates() {
    status('Checking for updates...');
    detail('');
    setPlay(false);
    if (!invoke('start_update')) {
      status('This page is the patcher\'s window. Download the patcher from the How to connect page.');
      return;
    }
    answered();
    waiting = setTimeout(function () {
      waiting = null;
      status('The patcher didn\'t answer.', true);
      detail('Try again, or close the patcher and start it again.');
      $('retry').style.display = '';
    }, answerTimeout);
  }

  $('play').onclick = function () { invoke('play'); };
  $('exit').onclick = function () { invoke('exit'); };
  $('retry').onclick = checkForUpdates;

  // Links open in the player's browser, not in the patcher's window.
  var links = document.getElementsByTagName('a');
  for (var i = 0; i < links.length; i++) {
    if (links[i].getAttribute('data-open') === null) {
      continue;
    }
    links[i].onclick = function () {
      return !invoke(JSON.stringify({ 'function': 'open_url', parameters: { url: this.href } }));
    };
  }

  checkForUpdates();
})();
