const logsDiv = document.getElementById('logs');
const statusIndicator = document.getElementById('status-indicator');
const tableBody = document.getElementById('task-table-body');
const eventSource = new EventSource('/events');

function formatRelativeTime(lastRunRfc3339) {
    if (!lastRunRfc3339) return 'Never';
    const lastRun = new Date(lastRunRfc3339);
    const now = new Date();
    const diffSeconds = Math.floor((now - lastRun) / 1000);

    if (diffSeconds < 0) return 'Just now';
    if (diffSeconds < 60) return `${diffSeconds} seconds ago`;
    if (diffSeconds < 3600) return `${Math.floor(diffSeconds / 60)} minutes ago`;
    if (diffSeconds < 86400) return `${Math.floor(diffSeconds / 3600)} hours ago`;
    return lastRun.toLocaleString();
}

eventSource.addEventListener('log', (event) => {
    const entry = document.createElement('div');
    entry.className = 'log-entry';
    
    // Check if it's an error message
    const isError = event.data.toLowerCase().includes('failed') || event.data.toLowerCase().includes('error');
    if (isError) {
        entry.classList.add('log-error');
    }
    
    const time = new Date().toLocaleTimeString();
    entry.innerHTML = `<span class="log-time">[${time}]</span> ${event.data}`;
    
    logsDiv.appendChild(entry);
    logsDiv.scrollTop = logsDiv.scrollHeight;

    while (logsDiv.childNodes.length > 1000) {
        logsDiv.removeChild(logsDiv.firstChild);
    }
});

eventSource.addEventListener('status', (event) => {
    const tasks = JSON.parse(event.data);
    let rows = '';
    
    tasks.forEach(task => {
        let statusClass = task.enabled ? 'status-enabled' : 'status-disabled';
        let statusText = task.enabled ? 'Active' : 'Paused';
        
        if (task.enabled && task.last_failed) {
            statusClass = 'status-disabled'; // Use red for failed
            statusText = 'Failed';
        }

        const lastRunStr = formatRelativeTime(task.last_run);
        const durationStr = (task.last_duration_ms !== null && task.last_duration_ms !== undefined) 
            ? `<span class='duration'>(${task.last_duration_ms}ms)</span>` 
            : '';
        
        rows += `
            <tr>
                <td><strong>${task.name}</strong></td>
                <td><span class='badge type-badge'>${task.task_type}</span></td>
                <td><code>${task.cron}</code></td>
                <td>${task.timezone}</td>
                <td class='time-cell'>${lastRunStr} ${durationStr}</td>
                <td><span class='status-pill ${statusClass}'>${statusText}</span></td>
            </tr>
        `;
    });
    
    if (tableBody) {
        tableBody.innerHTML = rows;
    }
});

// For backward compatibility or fallback messages without explicit event type
eventSource.onmessage = (event) => {
    if (!event.type || event.type === 'message') {
        const entry = document.createElement('div');
        entry.className = 'log-entry';
        const time = new Date().toLocaleTimeString();
        entry.innerHTML = `<span class="log-time">[${time}]</span> ${event.data}`;
        logsDiv.appendChild(entry);
        logsDiv.scrollTop = logsDiv.scrollHeight;
    }
};

eventSource.onopen = () => {
    statusIndicator.innerText = 'Connected';
    statusIndicator.style.color = 'var(--success)';
};

eventSource.onerror = () => {
    statusIndicator.innerText = 'Disconnected - retrying...';
    statusIndicator.style.color = 'var(--danger)';
};
