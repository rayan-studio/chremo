window.addEventListener('load', () => {
    setTimeout(() => {
        try {
            const ytIcons = Array.from(document.querySelectorAll('yt-icon, yt-icon-shape, ytd-guide-entry-renderer, #search-icon-legacy')).map(el => {
                const shadow = el.shadowRoot;
                const svgs = (shadow ? Array.from(shadow.querySelectorAll('svg')) : []).concat(Array.from(el.querySelectorAll('svg')));
                return {
                    tag: el.tagName,
                    id: el.id,
                    className: el.className,
                    hasShadow: !!shadow,
                    shadowHTML: shadow ? shadow.innerHTML.slice(0, 300) : null,
                    outerHTML: el.outerHTML.slice(0, 300),
                    compColor: window.getComputedStyle(el).color,
                    compFill: window.getComputedStyle(el).fill,
                    compDisplay: window.getComputedStyle(el).display,
                    compWidth: window.getComputedStyle(el).width,
                    compHeight: window.getComputedStyle(el).height,
                    svgCount: svgs.length,
                    svgs: svgs.map(s => ({
                        html: s.outerHTML.slice(0, 200),
                        fill: window.getComputedStyle(s).fill,
                        color: window.getComputedStyle(s).color,
                        width: window.getComputedStyle(s).width,
                        height: window.getComputedStyle(s).height,
                        display: window.getComputedStyle(s).display
                    }))
                };
            });
            fetch('http://127.0.0.1:9999/', {
                method: 'POST',
                headers: {'Content-Type': 'application/json'},
                body: JSON.stringify(ytIcons)
            });
        } catch(e) {
            fetch('http://127.0.0.1:9999/', {
                method: 'POST',
                body: 'ERROR: ' + e.toString()
            });
        }
    }, 3000);
});
