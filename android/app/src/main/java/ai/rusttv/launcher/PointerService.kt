package ai.rusttv.launcher

import android.accessibilityservice.AccessibilityService
import android.content.Context
import android.content.Intent
import android.graphics.Canvas
import android.graphics.Color
import android.graphics.Paint
import android.graphics.Path
import android.graphics.PixelFormat
import android.graphics.Rect
import android.graphics.RectF
import android.os.Handler
import android.os.Looper
import android.view.Gravity
import android.view.View
import android.view.WindowManager
import android.view.accessibility.AccessibilityEvent
import android.view.accessibility.AccessibilityNodeInfo
import android.view.accessibility.AccessibilityNodeInfo.AccessibilityAction

/**
 * The pointer in other apps (REMOTE.md § 5): a narrow accessibility service
 * that draws a cursor for a paired phone's touchpad and clicks or scrolls the
 * element under it.
 *
 * Android refuses injected touch gestures on a TV without a touchscreen, so a
 * click is the element's own accessibility action (owner decision
 * 2026-10-26). That needs `canRetrieveWindowContent`
 * (res/xml/pointer_service.xml), but this file reads only bounds, the
 * clickable/scrollable flags and the action list, of the elements under the
 * pointer, when a phone clicks or scrolls, the package of the app in front,
 * and the hint and text of a focused text field, which a phone types into:
 * nothing else, and it keeps nothing. No key filtering. Off until the
 * person turns it on in Android's accessibility settings; the launcher calls
 * it only while another app is in front. android/check-dex.sh keeps every
 * accessibility API in this file and allows only those reads.
 */
class PointerService : AccessibilityService() {
    private val main = Handler(Looper.getMainLooper())
    private var windows: WindowManager? = null
    private var cursor: View? = null
    // One full-screen, untouchable overlay; only the cursor's drawing moves
    // in it. Moving a small window instead cost a window relayout per step
    // (measured on the Philips: system_server ~20% and SurfaceFlinger ~15%
    // of a core while the pointer moved). The layer is gone while hidden.
    private var layer: View? = null
    private val place = WindowManager.LayoutParams(
        WindowManager.LayoutParams.MATCH_PARENT,
        WindowManager.LayoutParams.MATCH_PARENT,
        WindowManager.LayoutParams.TYPE_ACCESSIBILITY_OVERLAY,
        WindowManager.LayoutParams.FLAG_NOT_TOUCHABLE or
            WindowManager.LayoutParams.FLAG_NOT_FOCUSABLE or
            WindowManager.LayoutParams.FLAG_LAYOUT_IN_SCREEN or
            WindowManager.LayoutParams.FLAG_LAYOUT_NO_LIMITS,
        PixelFormat.TRANSLUCENT,
    ).apply { gravity = Gravity.TOP or Gravity.START }

    private var width = 1920f
    private var height = 1080f
    /** The click point, in screen pixels. */
    private var x = 960f
    private var y = 540f
    private val hide = Runnable { layer?.visibility = View.GONE }

    // Two fingers scroll the scrollable element under the pointer, one step
    // per SCROLL_STEP pixels of travel.
    private var owedX = 0f
    private var owedY = 0f

    override fun onServiceConnected() {
        val manager = getSystemService(Context.WINDOW_SERVICE) as WindowManager
        if (android.os.Build.VERSION.SDK_INT >= android.os.Build.VERSION_CODES.R) {
            val bounds = manager.maximumWindowMetrics.bounds
            width = bounds.width().toFloat()
            height = bounds.height().toFloat()
        } else {
            val size = android.graphics.Point()
            @Suppress("DEPRECATION")
            manager.defaultDisplay.getRealSize(size)
            width = size.x.toFloat()
            height = size.y.toFloat()
        }
        // Android may connect the service again while the launcher runs:
        // the cursor stays where it was.
        x = (lastX ?: width / 2).coerceIn(0f, width - 1)
        y = (lastY ?: height / 2).coerceIn(0f, height - 1)
        val view = Cursor(this, (height / 36).coerceAtLeast(16f))
        val root = android.widget.FrameLayout(this)
        root.addView(view, android.widget.FrameLayout.LayoutParams(
            android.widget.FrameLayout.LayoutParams.WRAP_CONTENT,
            android.widget.FrameLayout.LayoutParams.WRAP_CONTENT,
        ))
        root.visibility = View.GONE
        manager.addView(root, place)
        windows = manager
        cursor = view
        layer = root
        instance = this
        changed()
        // Connected again (Android pauses services, e.g. for UI automation):
        // read the app in front and its field at once.
        main.postDelayed(readFront, FRONT_SETTLE_MS)
        main.postDelayed(readField, FIELD_SETTLE_MS)
    }

    /** The package of the app in front, read when a window changes. */
    @Volatile
    private var front = ""
    /**
     * The app in front draws for the D-pad only: its window shows Android
     * (almost) no clickable element, so the pointer can't click there.
     */
    @Volatile
    private var dpad = false
    private val readFront = Runnable {
        val app = rootInActiveWindow?.packageName?.toString().orEmpty()
        if (app.isNotEmpty() && app != front) {
            front = app
            dpad = false
            changed()
            // Apps fill their window a moment after it opens.
            main.removeCallbacks(checkDpad)
            main.postDelayed(checkDpad, DPAD_CHECK_MS)
        }
    }
    // Almost nothing to click (one or two stray elements, like a hidden
    // close button) counts as drawn for the D-pad.
    private val checkDpad = Runnable { setDpad(clickables() in 0 until ENOUGH_CLICKABLE) }

    /** The focused text field as "label\nvalue", or "" when none. */
    @Volatile
    private var field = ""
    private val readField = Runnable {
        val node = focusedField()
        val now = node?.let { (it.hintText?.toString().orEmpty()) + "\n" + valueOf(it) }.orEmpty()
        if (now != field) {
            // The TV's keyboard covers the page: the phone types instead,
            // and the cursor gets out of the way.
            if (field.isEmpty() && now.isNotEmpty()) stop()
            field = now
            changed()
        }
    }

    // A window or the focus changed: once things settle, read which app is
    // in front and whether a text field has focus. The event isn't read.
    override fun onAccessibilityEvent(event: AccessibilityEvent?) {
        main.removeCallbacks(readFront)
        main.postDelayed(readFront, FRONT_SETTLE_MS)
        main.removeCallbacks(readField)
        main.postDelayed(readField, FIELD_SETTLE_MS)
    }

    // Some apps' fields (the TV browser's address bar) don't say they are
    // editable but still take text.
    private fun focusedField(): AccessibilityNodeInfo? =
        findFocus(AccessibilityNodeInfo.FOCUS_INPUT)?.takeIf {
            it.isEditable || AccessibilityAction.ACTION_SET_TEXT in it.actionList
        }

    private fun valueOf(node: AccessibilityNodeInfo): String =
        if (node.isShowingHintText) "" else node.text?.toString().orEmpty()

    /** After Enter: the field at [field] still has focus, so clicks the button beside it. */
    private fun goButtonIfStuck(field: Rect) {
        val node = focusedField() ?: return
        val box = Rect()
        node.getBoundsInScreen(box)
        if (box != field) return
        // The nearest clickable element to the right, level with the field,
        // within a few parents of it.
        var parent: AccessibilityNodeInfo? = node
        repeat(GO_BUTTON_PARENTS) { parent = parent?.parent }
        val queue = ArrayDeque<AccessibilityNodeInfo>()
        parent?.let(queue::add)
        var best: AccessibilityNodeInfo? = null
        var bestLeft = Int.MAX_VALUE
        var seen = 0
        while (queue.isNotEmpty() && seen < MAX_NODES) {
            val next = queue.removeFirst()
            seen++
            next.getBoundsInScreen(box)
            val level = box.top < field.centerY() && box.bottom > field.centerY()
            if (next.isClickable && next.isVisibleToUser && level && box.left >= field.right - 4 &&
                box.left < bestLeft && isGoButton(next)
            ) {
                best = next
                bestLeft = box.left
            }
            for (i in 0 until next.childCount) next.getChild(i)?.let(queue::add)
        }
        best?.performAction(AccessibilityNodeInfo.ACTION_CLICK)
    }

    /**
     * Whether a button beside a field runs it: named search or go, and not
     * voice search or clearing. Its name is read only for this.
     */
    private fun isGoButton(node: AccessibilityNodeInfo): Boolean {
        val name = ((node.contentDescription ?: "").toString() + " " + (node.text ?: "")).lowercase()
        // Short words count only whole ("go", not "Google").
        val words = name.split(Regex("[^\\p{L}\\p{N}]+")).toSet()
        val named = GO.any { if (it.length <= 3) it in words else it in name }
        return named && NOT_GO.none { it in name }
    }

    /** A phone's edit to the focused field: 0 insert, 1 delete, 2 Enter. */
    private fun type(op: Int, text: String, count: Int) {
        val node = focusedField() ?: return
        if (op == 2) {
            // The keyboard's Enter. Android sends it as "done", which some
            // fields ignore (the TV browser's address bar waits for "search"):
            // if the field still has focus a moment later, its search or go
            // button next to it is clicked instead.
            if (android.os.Build.VERSION.SDK_INT >= android.os.Build.VERSION_CODES.R) {
                node.performAction(AccessibilityAction.ACTION_IME_ENTER.id)
            }
            val box = Rect()
            node.getBoundsInScreen(box)
            main.postDelayed({ goButtonIfStuck(box) }, ENTER_CHECK_MS)
            return
        }
        val now = valueOf(node)
        val next = if (op == 0) {
            now + text
        } else {
            val keep = now.offsetByCodePoints(now.length, -count.coerceAtMost(now.codePointCount(0, now.length)))
            now.substring(0, keep)
        }
        val args = android.os.Bundle()
        args.putCharSequence(AccessibilityNodeInfo.ACTION_ARGUMENT_SET_TEXT_CHARSEQUENCE, next)
        node.performAction(AccessibilityNodeInfo.ACTION_SET_TEXT, args)
        val end = android.os.Bundle()
        end.putInt(AccessibilityNodeInfo.ACTION_ARGUMENT_SELECTION_START_INT, next.length)
        end.putInt(AccessibilityNodeInfo.ACTION_ARGUMENT_SELECTION_END_INT, next.length)
        node.performAction(AccessibilityNodeInfo.ACTION_SET_SELECTION, end)
    }

    private fun setDpad(value: Boolean) {
        if (value != dpad) {
            dpad = value
            changed()
        }
    }

    /** How many clickable elements the app in front shows, up to a few. */
    private fun clickables(): Int {
        val root = rootInActiveWindow ?: return -1
        val queue = ArrayDeque<AccessibilityNodeInfo>()
        queue.add(root)
        var seen = 0
        var found = 0
        while (queue.isNotEmpty() && seen < MAX_NODES && found < ENOUGH_CLICKABLE) {
            val node = queue.removeFirst()
            seen++
            if (node.isVisibleToUser && (node.isClickable || AccessibilityAction.ACTION_CLICK in node.actionList)) found++
            for (i in 0 until node.childCount) node.getChild(i)?.let(queue::add)
        }
        return found
    }

    override fun onInterrupt() {}

    override fun onUnbind(intent: Intent?): Boolean {
        close()
        return super.onUnbind(intent)
    }

    override fun onDestroy() {
        close()
        super.onDestroy()
    }

    private fun close() {
        if (instance !== this) return
        softKeyboardController.showMode = SHOW_MODE_AUTO
        instance = null
        main.removeCallbacksAndMessages(null)
        layer?.let { windows?.removeViewImmediate(it) }
        layer = null
        cursor = null
        windows = null
        changed()
    }

    private fun act(dx: Int, dy: Int, action: Int) {
        when (action) {
            MOVE -> move(dx, dy)
            CLICK -> click()
            SCROLL -> scroll(dx, dy)
        }
    }

    private fun move(dx: Int, dy: Int) {
        x = (x + speed(dx)).coerceIn(0f, width - 1)
        y = (y + speed(dy)).coerceIn(0f, height - 1)
        lastX = x
        lastY = y
        show()
    }

    /** Phone pixels to screen pixels, faster for quick swipes. */
    private fun speed(delta: Int): Float {
        val scale = width / 1920f
        return delta * scale * (1f + (kotlin.math.abs(delta) / 40f).coerceAtMost(1.5f))
    }

    private fun show() {
        val view = cursor ?: return
        view.translationX = x + OFFSET
        view.translationY = y + OFFSET
        if (layer?.visibility != View.VISIBLE) layer?.visibility = View.VISIBLE
        main.removeCallbacks(hide)
        main.postDelayed(hide, HIDE_AFTER_MS)
    }

    private fun click() {
        show()
        val clickable = { node: AccessibilityNodeInfo -> node.isClickable || AccessibilityAction.ACTION_CLICK in node.actionList }
        // A little forgiving: just beside an element (the gap between two
        // rows) still clicks the nearest one.
        val node = NEAR.firstNotNullOfOrNull { (dx, dy) ->
            under((x + dx).coerceIn(0f, width - 1), (y + dy).coerceIn(0f, height - 1), clickable)
        }
        // A whole clickable row around a card that only takes focus (TV apps
        // that click through OK): focus the card and press OK.
        val card = under(x, y) { it.isFocusable && !clickable(it) }
        if (node != null && card != null && area(node) > area(card) * 4 &&
            android.os.Build.VERSION.SDK_INT >= android.os.Build.VERSION_CODES.TIRAMISU
        ) {
            card.performAction(AccessibilityNodeInfo.ACTION_FOCUS)
            main.postDelayed({ performGlobalAction(GLOBAL_ACTION_DPAD_CENTER) }, FOCUS_THEN_OK_MS)
            return
        }
        if (node != null) {
            val clicked = node.performAction(AccessibilityNodeInfo.ACTION_CLICK)
            // Some TV apps (Compose for TV cards) refuse clicks from outside
            // but answer OK on the focused element.
            if (!clicked && android.os.Build.VERSION.SDK_INT >= android.os.Build.VERSION_CODES.TIRAMISU) {
                node.performAction(AccessibilityNodeInfo.ACTION_FOCUS)
                main.postDelayed({ performGlobalAction(GLOBAL_ACTION_DPAD_CENTER) }, FOCUS_THEN_OK_MS)
            }
        } else {
            // A field that takes focus but no click (some text fields).
            val field = under(x, y) { it.isFocusable }
            field?.performAction(AccessibilityNodeInfo.ACTION_FOCUS)
            // Nothing on screen to click at all: the phone switches to arrows.
            if (field == null && clickables() in 0 until ENOUGH_CLICKABLE) setDpad(true)
        }
    }

    private fun scroll(dx: Int, dy: Int) {
        show()
        owedX += dx
        owedY += dy
        val vertical = kotlin.math.abs(owedY) >= kotlin.math.abs(owedX)
        val owed = if (vertical) owedY else owedX
        if (kotlin.math.abs(owed) < SCROLL_STEP) return
        owedX = 0f
        owedY = 0f
        // Fingers moving up show what is below, as on a phone.
        val forward = owed < 0
        val action = when {
            vertical && forward -> AccessibilityAction.ACTION_SCROLL_DOWN
            vertical -> AccessibilityAction.ACTION_SCROLL_UP
            forward -> AccessibilityAction.ACTION_SCROLL_RIGHT
            else -> AccessibilityAction.ACTION_SCROLL_LEFT
        }
        val path = scrollablesAt(x, y)
        // The nearest element that scrolls this way (a page around a slider
        // for up and down, the slider for left and right).
        path.firstOrNull { action in it.actionList }?.let {
            it.performAction(action.id)
            return
        }
        // Without a direction: the nearest list whose items run this way
        // (stacked for up and down, side by side for left and right), else
        // by shape.
        val box = Rect()
        val shaped = path.firstOrNull { runsVertically(it) == vertical }
            ?: path.firstOrNull {
                if (runsVertically(it) != null) return@firstOrNull false
                it.getBoundsInScreen(box)
                if (vertical) box.height() * 2 >= box.width() else box.width() >= box.height() * 3
            }
        shaped?.performAction(
            if (forward) AccessibilityNodeInfo.ACTION_SCROLL_FORWARD
            else AccessibilityNodeInfo.ACTION_SCROLL_BACKWARD,
        )
    }

    private fun area(node: AccessibilityNodeInfo): Long {
        val box = Rect()
        node.getBoundsInScreen(box)
        return box.width().toLong() * box.height()
    }

    /**
     * Whether a list's items are stacked (true) or side by side (false), from
     * its first few children; null when it can't tell.
     */
    private fun runsVertically(list: AccessibilityNodeInfo): Boolean? {
        val box = Rect()
        var minX = Int.MAX_VALUE
        var maxX = Int.MIN_VALUE
        var minY = Int.MAX_VALUE
        var maxY = Int.MIN_VALUE
        var seen = 0
        for (i in 0 until minOf(list.childCount, 6)) {
            val child = list.getChild(i) ?: continue
            child.getBoundsInScreen(box)
            if (box.isEmpty) continue
            minX = minOf(minX, box.left)
            maxX = maxOf(maxX, box.left)
            minY = minOf(minY, box.top)
            maxY = maxOf(maxY, box.top)
            seen++
        }
        if (seen < 2 || (maxX - minX == 0 && maxY - minY == 0)) return null
        return maxY - minY > maxX - minX
    }

    /**
     * The element of the app in front at (x, y) that [wanted] accepts: the
     * deepest such element under the point, top-most layer first on ties.
     */
    private fun under(x: Float, y: Float, wanted: (AccessibilityNodeInfo) -> Boolean): AccessibilityNodeInfo? {
        var best: AccessibilityNodeInfo? = null
        var bestDepth = -1
        for (path in pathsAt(x, y)) {
            // Root first: the last match is the deepest on this chain.
            path.withIndex().lastOrNull { (_, node) -> node.isEnabled && wanted(node) }?.let { (depth, node) ->
                if (depth > bestDepth) {
                    best = node
                    bestDepth = depth
                }
            }
        }
        return best
    }

    /** The scrollable elements at (x, y), nearest first, from every layer. */
    private fun scrollablesAt(x: Float, y: Float): List<AccessibilityNodeInfo> =
        pathsAt(x, y)
            .flatMap { path -> path.withIndex().filter { (_, node) -> node.isEnabled && node.isScrollable } }
            .sortedByDescending { it.index }
            .map { it.value }
            .distinct()

    /**
     * Every chain of visible elements from the root to a deepest element at
     * (x, y), top-most layer first. All layers are searched: pages often
     * draw an empty full-screen layer over the real content.
     */
    private fun pathsAt(x: Float, y: Float): List<List<AccessibilityNodeInfo>> {
        val px = x.toInt()
        val py = y.toInt()
        val box = Rect()
        val paths = ArrayList<List<AccessibilityNodeInfo>>()
        var seen = 0
        val path = ArrayList<AccessibilityNodeInfo>()
        fun visit(node: AccessibilityNodeInfo) {
            if (seen++ > MAX_NODES || path.size >= MAX_DEPTH) return
            path.add(node)
            var leaf = true
            // The last child is drawn on top: tried first.
            for (i in node.childCount - 1 downTo 0) {
                val child = node.getChild(i) ?: continue
                child.getBoundsInScreen(box)
                if (child.isVisibleToUser && box.contains(px, py)) {
                    leaf = false
                    visit(child)
                }
            }
            if (leaf) paths.add(path.toList())
            path.removeAt(path.size - 1)
        }
        rootInActiveWindow?.let(::visit)
        return paths
    }

    private fun press(code: Int) {
        val audio = getSystemService(Context.AUDIO_SERVICE) as android.media.AudioManager
        val media = { keycode: Int ->
            audio.dispatchMediaKeyEvent(android.view.KeyEvent(android.view.KeyEvent.ACTION_DOWN, keycode))
            audio.dispatchMediaKeyEvent(android.view.KeyEvent(android.view.KeyEvent.ACTION_UP, keycode))
        }
        val volume = { direction: Int ->
            audio.adjustStreamVolume(
                android.media.AudioManager.STREAM_MUSIC,
                direction,
                // The TV's own volume bar, as for the remote's keys: one step
                // of 60 is hard to hear without it. It takes focus, so the
                // launcher never counts it as the app in front.
                android.media.AudioManager.FLAG_SHOW_UI,
            )
        }
        val dpad = android.os.Build.VERSION.SDK_INT >= android.os.Build.VERSION_CODES.TIRAMISU
        // Arrows and OK move the app's own highlight: the cursor gets out of the way.
        if (code <= 4) stop()
        when (code) {
            0 -> if (dpad) performGlobalAction(GLOBAL_ACTION_DPAD_UP)
            1 -> if (dpad) performGlobalAction(GLOBAL_ACTION_DPAD_DOWN)
            2 -> if (dpad) performGlobalAction(GLOBAL_ACTION_DPAD_LEFT)
            3 -> if (dpad) performGlobalAction(GLOBAL_ACTION_DPAD_RIGHT)
            4 -> if (dpad) performGlobalAction(GLOBAL_ACTION_DPAD_CENTER)
            5 -> performGlobalAction(GLOBAL_ACTION_BACK)
            6 -> performGlobalAction(GLOBAL_ACTION_HOME)
            7 -> media(android.view.KeyEvent.KEYCODE_MEDIA_PLAY_PAUSE)
            8 -> media(android.view.KeyEvent.KEYCODE_MEDIA_REWIND)
            9 -> media(android.view.KeyEvent.KEYCODE_MEDIA_FAST_FORWARD)
            10 -> volume(android.media.AudioManager.ADJUST_RAISE)
            11 -> volume(android.media.AudioManager.ADJUST_LOWER)
            12 -> volume(android.media.AudioManager.ADJUST_TOGGLE_MUTE)
        }
    }

    private fun stop() {
        main.removeCallbacks(hide)
        layer?.visibility = View.GONE
        owedX = 0f
        owedY = 0f
    }

    /**
     * The cursor: a round dot with its top-left corner pointed; the click
     * point is just above-left of its tip.
     */
    private class Cursor(context: Context, private val size: Float) : View(context) {
        private val fill = Paint(Paint.ANTI_ALIAS_FLAG).apply {
            color = Color.WHITE
            setShadowLayer(size / 6, 0f, size / 12, 0x80000000.toInt())
        }
        private val edge = Paint(Paint.ANTI_ALIAS_FLAG).apply {
            style = Paint.Style.STROKE
            strokeWidth = (size / 14).coerceAtLeast(2f)
            color = 0xFF202124.toInt()
        }
        private val shape = Path()

        init {
            setLayerType(LAYER_TYPE_SOFTWARE, null)
        }

        override fun onMeasure(widthSpec: Int, heightSpec: Int) {
            val side = (size * 1.5f).toInt()
            setMeasuredDimension(side, side)
        }

        override fun onDraw(canvas: Canvas) {
            val inset = edge.strokeWidth
            val r = size / 2
            val box = RectF(inset, inset, size, size)
            shape.reset()
            shape.addRoundRect(box, floatArrayOf(0f, 0f, r, r, r, r, r, r), Path.Direction.CW)
            canvas.drawPath(shape, fill)
            canvas.drawPath(shape, edge)
        }
    }

    companion object {
        private const val MOVE = 0
        private const val CLICK = 1
        private const val SCROLL = 2
        /** The cursor is drawn this far right of and below the click point. */
        private const val OFFSET = 2
        private const val HIDE_AFTER_MS = 6_000L
        /** Phone pixels (doubled by the page) of two-finger travel per scroll step. */
        private const val SCROLL_STEP = 160f
        /** Deeper trees than this are cut short (a guard, not a real limit). */
        private const val MAX_DEPTH = 64
        private const val FRONT_SETTLE_MS = 250L
        private const val FIELD_SETTLE_MS = 120L
        private const val ENTER_CHECK_MS = 400L
        private const val GO_BUTTON_PARENTS = 3
        private const val FOCUS_THEN_OK_MS = 80L
        /** Where a click looks, nearest first, when nothing is right under the cursor. */
        private val NEAR = listOf(
            0f to 0f, 0f to -12f, 0f to 12f, -12f to 0f, 12f to 0f,
            0f to -24f, 0f to 24f, -24f to 0f, 24f to 0f, 0f to -32f, 0f to 32f,
        )
        /** Words, in the launcher's languages, of a button that runs a search or address. */
        private val GO = listOf(
            "search", "go", "submit", "enter", "пошук", "шукати", "знайти", "поиск", "найти", "искать",
            "suche", "suchen", "buscar", "búsqueda", "recherche", "chercher", "cerca", "zoek", "szukaj",
            "wyszukaj", "ara", "ir", "aller", "vai", "gehe", "بحث", "חיפוש", "खोज", "cari", "検索", "검색",
            "ค้นหา", "tìm", "搜索", "搜尋",
        )
        /** Words of buttons beside a field that never run it. */
        private val NOT_GO = listOf(
            "clear", "erase", "delete", "очист", "видал", "удал", "стер", "lösch", "borrar", "effacer",
            "cancella", "wissen", "usuń", "temizle", "voice", "mic", "голос", "микроф", "мікроф", "sprach",
            "voz", "vocal", "voce", "spraak", "głos", "sesli",
        )
        private const val DPAD_CHECK_MS = 1_500L
        private const val MAX_NODES = 600
        private const val ENOUGH_CLICKABLE = 3

        @Volatile
        private var instance: PointerService? = null
        /** Where the cursor was, for a service connected again. */
        private var lastX: Float? = null
        private var lastY: Float? = null
        private val handler = Handler(Looper.getMainLooper())

        /** "ready" while the person has the service on, else "off". */
        fun state(): String = if (instance != null) "ready" else "off"

        /** The package of the app in front, or "" (service off, not known yet). */
        fun app(): String = instance?.front.orEmpty()

        /** The focused text field as "label\nvalue", or "". */
        fun field(): String = instance?.field.orEmpty()

        /** Types into the focused field: 0 insert [text], 1 delete [count] characters, 2 Enter. */
        fun type(op: Int, text: String, count: Int) {
            handler.post { instance?.type(op, text, count) }
        }

        /** Hides Android's on-screen keyboard (while a phone types) or lets it show. */
        fun hideKeyboard(hide: Boolean) {
            handler.post {
                instance?.softKeyboardController?.showMode =
                    if (hide) AccessibilityService.SHOW_MODE_HIDDEN else AccessibilityService.SHOW_MODE_AUTO
            }
        }

        /** Whether the app in front draws for the D-pad only. */
        fun dpad(): Boolean = instance?.dpad == true

        /** Moves (0), clicks (1) or scrolls (2), on the main thread. */
        fun post(dx: Int, dy: Int, action: Int) {
            handler.post { instance?.act(dx, dy, action) }
        }

        /**
         * Presses a key in the app in front: 0–4 up, down, left, right, OK;
         * 5 Back, 6 Home, 7 play/pause, 8 rewind, 9 fast forward, 10 volume
         * up, 11 down, 12 mute.
         */
        fun key(code: Int) {
            handler.post { instance?.press(code) }
        }

        /** Hides the cursor and drops a scroll in progress. */
        fun hide() {
            handler.post { instance?.stop() }
        }

        /** Tells the launcher, when its native side is loaded. */
        private fun changed() {
            try {
                nativeChanged()
            } catch (_: UnsatisfiedLinkError) {
                // The launcher hasn't started in this process: it reads the
                // state when it does.
            }
        }

        @JvmStatic
        external fun nativeChanged()
    }
}
