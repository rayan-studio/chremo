window.addEventListener('load', () => {
    setTimeout(() => {
        try {
            const svgs = Array.from(document.querySelectorAll('svg'));
            const data = svgs.map((s, idx) => {
                const cs = window.getComputedStyle(s);
                const paths = Array.from(s.querySelectorAll('path')).map(p => ({
                    attrFill: p.getAttribute('fill'),
                    compFill: window.getComputedStyle(p).fill,
                    compColor: window.getComputedStyle(p).color
                }));
                let parent = s.parentElement;
                return {
                    idx: idx,
                    cls: s.className?.baseVal || s.className,
                    aria: s.getAttribute('aria-label') || parent?.getAttribute('aria-label'),
                    fill: cs.fill,
                    color: cs.color,
                    html: s.outerHTML.slice(0, 300),
                    parentTag: parent?.tagName,
                    parentCls: parent?.className,
                    parentColor: parent ? window.getComputedStyle(parent).color : '',
                    paths: paths
                };
            });
            fetch('http://127.0.0.1:9997/', {
                method: 'POST',
                headers: {'Content-Type': 'application/json'},
                body: JSON.stringify(data)
            });
        } catch(e) {
            fetch('http://127.0.0.1:9997/', {
                method: 'POST',
                body: 'ERROR: ' + e.toString()
            });
        }
    }, 2000);
});
