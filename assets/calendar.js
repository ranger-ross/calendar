(function () {
  'use strict';

  var body = document.body;
  var dateNode = document.getElementById('date');
  var clockNode = document.getElementById('clock');
  var calendarNode = document.getElementById('calendar');
  var statusNode = document.getElementById('status');
  var statusMessageNode = document.getElementById('status-message');
  var syncAgeNode = document.getElementById('sync-age');
  var initialSync = syncAgeNode.getAttribute('data-updated-at-ms');
  var lastSynced = initialSync === null ? null : Number(initialSync);
  var themeButton = document.getElementById('theme-toggle');
  var serverNow = Number(body.getAttribute('data-now-ms'));
  var offsetSeconds = Number(body.getAttribute('data-offset-seconds'));
  var receivedAt = new Date().getTime();
  var serverStatus = statusMessageNode.textContent;
  var connectionError = '';
  var requestPending = false;
  var lastHtml = null;
  var previousDay = null;
  var weekdays = ['Sunday', 'Monday', 'Tuesday', 'Wednesday', 'Thursday', 'Friday', 'Saturday'];
  var months = ['January', 'February', 'March', 'April', 'May', 'June', 'July', 'August', 'September', 'October', 'November', 'December'];

  try {
    if (localStorage.getItem('calendar-theme') === 'light') {
      document.documentElement.setAttribute('data-theme', 'light');
    }
  } catch (error) {
    // Storage can be disabled in kiosk or private browsing mode.
  }

  function pad(value) {
    return value < 10 ? '0' + value : String(value);
  }

  function showStatus() {
    statusMessageNode.textContent = connectionError ? connectionError + ' ' + serverStatus : serverStatus;
    statusNode.className = connectionError || serverStatus.indexOf('failed:') !== -1 || serverStatus.indexOf('unavailable:') !== -1 ? 'has-error' : '';
  }

  function updateSyncAge() {
    var text = '';
    var seconds;
    var count;
    var unit;
    if (lastSynced !== null) {
      seconds = Math.max(0, Math.floor((serverNow + new Date().getTime() - receivedAt - lastSynced) / 1000));
      if (seconds < 60) {
        text = ' less than a minute ago';
      } else {
        unit = seconds < 3600 ? 'minute' : seconds < 86400 ? 'hour' : 'day';
        count = Math.floor(seconds / (unit === 'minute' ? 60 : unit === 'hour' ? 3600 : 86400));
        text = ' ' + count + ' ' + unit + (count === 1 ? '' : 's') + ' ago';
      }
    }
    if (syncAgeNode.textContent !== text) {
      syncAgeNode.textContent = text;
    }
  }

  function tick() {
    var now = new Date(serverNow + new Date().getTime() - receivedAt + offsetSeconds * 1000);
    var hours = now.getUTCHours();
    var day = now.getUTCFullYear() + '-' + now.getUTCMonth() + '-' + now.getUTCDate();
    var clock = (hours % 12 || 12) + ':' + pad(now.getUTCMinutes()) + '<sup>' + pad(now.getUTCSeconds()) + '</sup> <span class="meridiem">' + (hours < 12 ? 'am' : 'pm') + '</span>';
    var date = weekdays[now.getUTCDay()] + ', ' + months[now.getUTCMonth()] + ' ' + now.getUTCDate() + ', ' + now.getUTCFullYear();
    if (clockNode.innerHTML !== clock) {
      clockNode.innerHTML = clock;
    }
    if (dateNode.textContent !== date) {
      dateNode.textContent = date;
    }
    if (previousDay !== null && previousDay !== day) {
      previousDay = day;
      poll();
    }
    previousDay = day;
    updateSyncAge();
  }

  function updateCalendar(html) {
    var normalized;
    var oldDays;
    var newDays;
    var positions = [];
    var i;
    if (html === lastHtml) {
      return;
    }
    normalized = document.createElement('div');
    normalized.innerHTML = html;
    if (normalized.innerHTML !== calendarNode.innerHTML) {
      oldDays = calendarNode.getElementsByClassName('events');
      for (i = 0; i < oldDays.length; i += 1) {
        positions.push(oldDays[i].scrollTop);
      }
      calendarNode.innerHTML = html;
      newDays = calendarNode.getElementsByClassName('events');
      for (i = 0; i < newDays.length && i < positions.length; i += 1) {
        newDays[i].scrollTop = positions[i];
      }
    }
    lastHtml = html;
  }

  function poll() {
    var xhr;
    if (requestPending) {
      return;
    }
    requestPending = true;
    xhr = new XMLHttpRequest();

    function failed(message) {
      requestPending = false;
      connectionError = message + ' Keeping the displayed calendar; retrying automatically.';
      showStatus();
    }

    xhr.open('GET', '/api/view', true);
    xhr.timeout = 20000;
    xhr.setRequestHeader('Accept', 'application/json');
    xhr.onload = function () {
      var data;
      if (xhr.status < 200 || xhr.status >= 300) {
        failed('Calendar connection failed (HTTP ' + xhr.status + ').');
        return;
      }
      try {
        data = JSON.parse(xhr.responseText);
        if (!data || typeof data.html !== 'string' || typeof data.date !== 'string' || typeof data.clock !== 'string' || typeof data.status !== 'string' || typeof data.now_ms !== 'number' || !isFinite(data.now_ms) || typeof data.offset_seconds !== 'number' || !isFinite(data.offset_seconds) || (data.updated_at_ms !== null && (typeof data.updated_at_ms !== 'number' || !isFinite(data.updated_at_ms)))) {
          throw new Error('Invalid calendar response');
        }
      } catch (error) {
        failed('The server returned an unreadable calendar update.');
        return;
      }
      updateCalendar(data.html);
      serverNow = data.now_ms;
      offsetSeconds = data.offset_seconds;
      receivedAt = new Date().getTime();
      serverStatus = data.status;
      lastSynced = data.updated_at_ms;
      connectionError = '';
      requestPending = false;
      showStatus();
      tick();
    };
    xhr.onerror = function () { failed('Cannot reach the calendar server.'); };
    xhr.ontimeout = function () { failed('The calendar server took too long to respond.'); };
    xhr.onabort = function () { failed('The calendar update was interrupted.'); };
    try {
      xhr.send(null);
    } catch (error) {
      failed('Cannot reach the calendar server.');
    }
  }

  function updateThemeButton() {
    var light = document.documentElement.getAttribute('data-theme') === 'light';
    themeButton.textContent = light ? 'Dark theme' : 'Light theme';
    themeButton.setAttribute('aria-pressed', light ? 'true' : 'false');
  }

  themeButton.onclick = function () {
    var theme = document.documentElement.getAttribute('data-theme') === 'light' ? 'dark' : 'light';
    document.documentElement.setAttribute('data-theme', theme);
    try {
      localStorage.setItem('calendar-theme', theme);
    } catch (error) {
      // The theme still works when storage is unavailable.
    }
    updateThemeButton();
  };

  document.addEventListener('visibilitychange', function () {
    if (!document.hidden) {
      tick();
      poll();
    }
  }, false);
  window.addEventListener('pageshow', function () {
    tick();
    poll();
  }, false);
  updateThemeButton();
  showStatus();
  tick();
  window.setInterval(tick, 1000);
  window.setInterval(poll, 60000);
}());
