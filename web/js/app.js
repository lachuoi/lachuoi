const logsDiv = document.getElementById('logs');
const statusIndicator = document.getElementById('status-indicator');
const tableBody = document.getElementById('task-table-body');
const eventSource = new EventSource('/events');

let latestTasks = [];
let currentSortColumn = 'name';
let currentSortDirection = 'asc';

function setSort(column) {
    if (currentSortColumn === column) {
        currentSortDirection = currentSortDirection === 'asc' ? 'desc' : 'asc';
    } else {
        currentSortColumn = column;
        currentSortDirection = 'asc';
    }
    renderTable();
}

function updateSortIndicators() {
    const columns = ['name', 'task_type', 'cron', 'timezone', 'last_run', 'enabled'];
    columns.forEach(col => {
        const indicator = document.getElementById(`sort-${col}`);
        if (!indicator) return;
        
        if (currentSortColumn === col) {
            indicator.innerText = currentSortDirection === 'asc' ? '↑' : '↓';
            indicator.className = 'text-blue-500 font-bold opacity-100';
        } else {
            indicator.innerText = '↕';
            indicator.className = 'opacity-0 group-hover:opacity-100 transition-opacity';
        }
    });
}

function renderTable() {
    if (!latestTasks || latestTasks.length === 0) return;

    // 1. Sort the data
    const sortedTasks = [...latestTasks].sort((a, b) => {
        let valA = a[currentSortColumn];
        let valB = b[currentSortColumn];

        // Special handling for dates
        if (currentSortColumn === 'last_run') {
            valA = valA ? new Date(valA).getTime() : 0;
            valB = valB ? new Date(valB).getTime() : 0;
        } else {
            // Handle nulls for strings/booleans
            if (valA === null || valA === undefined) valA = '';
            if (valB === null || valB === undefined) valB = '';
        }

        if (valA < valB) return currentSortDirection === 'asc' ? -1 : 1;
        if (valA > valB) return currentSortDirection === 'asc' ? 1 : -1;
        return 0;
    });

    // 2. Generate HTML
    let rows = '';
    sortedTasks.forEach(task => {
        let statusClass = task.enabled ? 'bg-green-50 text-green-700 border-green-200' : 'bg-red-50 text-red-700 border-red-200';
        let statusText = task.enabled ? 'Active' : 'Paused';
        
        if (task.enabled && task.last_failed) {
            statusClass = 'bg-red-50 text-red-700 border-red-200';
            statusText = 'Failed';
        }

        const lastRunStr = formatRelativeTime(task.last_run);
        const durationStr = (task.last_duration_ms !== null && task.last_duration_ms !== undefined) 
            ? `<span class='ml-1 text-xs text-blue-600 font-bold'>(${task.last_duration_ms}ms)</span>` 
            : '';
        
        const toggleBtn = task.enabled 
            ? `<button class='px-3 py-1 text-xs font-semibold text-red-600 bg-red-50 border border-red-200 rounded-md hover:bg-red-600 hover:text-white transition-all duration-200' onclick='toggleTask("${task.id}", false)'>Disable</button>`
            : `<button class='px-3 py-1 text-xs font-semibold text-green-600 bg-green-50 border border-green-200 rounded-md hover:bg-green-600 hover:text-white transition-all duration-200' onclick='toggleTask("${task.id}", true)'>Enable</button>`;

        rows += `
            <tr class="border-b border-gray-100 hover:bg-gray-50 transition-colors">
                <td class="px-4 py-3 align-middle text-sm font-bold text-gray-900">${task.name}</td>
                <td class="px-4 py-3 align-middle text-xs"><span class="bg-gray-100 text-gray-600 px-2 py-1 rounded font-medium uppercase tracking-wider">${task.task_type}</span></td>
                <td class="px-4 py-3 align-middle font-mono text-blue-600 text-xs">${task.cron}</td>
                <td class="px-4 py-3 align-middle text-gray-600 text-sm">${task.timezone}</td>
                <td class="px-4 py-3 align-middle text-sm text-gray-500" title="${task.last_run || 'Never'}">${lastRunStr} ${durationStr}</td>
                <td class="px-4 py-3 align-middle"><span class="px-2 py-1 text-[10px] uppercase font-bold rounded-full border ${statusClass}">${statusText}</span></td>
                <td class="px-4 py-3 align-middle">${toggleBtn}</td>
            </tr>
        `;
    });
    
    if (tableBody) {
        tableBody.innerHTML = rows;
    }
    
    updateSortIndicators();
}

function formatRelativeTime(lastRunRfc3339) {
    if (!lastRunRfc3339) return 'Never';
    const lastRun = new Date(lastRunRfc3339);
    const now = new Date();
    const diffSeconds = Math.floor((now - lastRun) / 1000);

    if (isNaN(lastRun.getTime())) return 'Invalid Date';
    if (diffSeconds < 0) return 'Just now';
    if (diffSeconds < 60) return `${diffSeconds}s ago`;
    if (diffSeconds < 3600) return `${Math.floor(diffSeconds / 60)}m ago`;
    if (diffSeconds < 86400) return `${Math.floor(diffSeconds / 3600)}h ago`;
    return lastRun.toLocaleTimeString();
}

function pruneLogs() {
    while (logsDiv.childNodes.length > 1000) {
        logsDiv.removeChild(logsDiv.firstChild);
    }
}

eventSource.addEventListener('log', (event) => {
    const entry = document.createElement('div');
    entry.className = 'mb-1 flex gap-3 text-xs md:text-sm';
    
    // Check if it's an error message
    const isError = event.data.toLowerCase().includes('failed') || event.data.toLowerCase().includes('error');
    const textColor = isError ? 'text-red-400 font-medium' : 'text-slate-300';
    
    const time = new Date().toLocaleTimeString();
    entry.innerHTML = `
        <span class="text-slate-500 shrink-0 font-medium select-none">[${time}]</span> 
        <span class="${textColor}">${event.data}</span>
    `;
    
    logsDiv.appendChild(entry);
    logsDiv.scrollTop = logsDiv.scrollHeight;
    pruneLogs();
});

eventSource.addEventListener('status', (event) => {
    latestTasks = JSON.parse(event.data);
    renderTable();
});

async function toggleTask(taskId, enabled) {
    try {
        const response = await fetch(`/tasks/${taskId}/toggle`, {
            method: 'POST',
            headers: {
                'Content-Type': 'application/json'
            },
            body: JSON.stringify({ enabled })
        });
        
        if (!response.ok) {
            alert('Failed to update task status');
        }
    } catch (error) {
        console.error('Error toggling task:', error);
        alert('Error toggling task status');
    }
}

// For backward compatibility or fallback messages without explicit event type
eventSource.onmessage = (event) => {
    if (!event.type || event.type === 'message') {
        const entry = document.createElement('div');
        entry.className = 'mb-1 flex gap-3 text-xs md:text-sm';
        const time = new Date().toLocaleTimeString();
        entry.innerHTML = `
            <span class="text-slate-500 shrink-0 font-medium select-none">[${time}]</span> 
            <span class="text-slate-400">${event.data}</span>
        `;
        logsDiv.appendChild(entry);
        logsDiv.scrollTop = logsDiv.scrollHeight;
        pruneLogs();
    }
};

eventSource.onopen = () => {
    statusIndicator.innerText = 'Connected';
    statusIndicator.className = 'text-xs font-bold text-emerald-400';
};

eventSource.onerror = () => {
    statusIndicator.innerText = 'Disconnected - retrying...';
    statusIndicator.className = 'text-xs font-bold text-red-400';
};
