import UIKit

/// One provider in the model picker: its tab and its models, loaded when the
/// picker opens.
struct ModelPickerTab {
    let harness: String
    let label: String
    /// Under the list: what picking here does to the chat.
    var footer: String? = nil
    let load: () async -> [ModelPickerRow]
}

struct ModelPickerRow: Equatable {
    let id: String
    let label: String
    let detail: String?
}

/// The model picker: a glass sheet sized to its list, with a provider
/// switcher over swipeable pages (the current provider first) when the chat
/// may change provider. Only one provider: no switcher, the provider rides in
/// the title and `note` says why it's the only one.
final class ModelPickerViewController: UIViewController, UIScrollViewDelegate {
    private let tabs: [ModelPickerTab]
    private let host: (name: String, symbol: String)
    private let note: String?
    private let onPick: (_ harness: String, _ model: ModelPickerRow) -> Void

    private var selected: Int
    private var pages: [Page] = []
    private let pager = UIScrollView()
    private var switcher: ProviderSwitcher?
    private weak var sheet: UISheetPresentationController?
    private var picked = false

    /// The picker in its sheet, ready to present.
    static func sheet(tabs: [ModelPickerTab], harness: String, model: String?, host: (name: String, symbol: String), note: String? = nil, onPick: @escaping (String, ModelPickerRow) -> Void) -> UIViewController {
        let picker = ModelPickerViewController(tabs: tabs, harness: harness, model: model, host: host, note: note, onPick: onPick)
        let nav = UINavigationController(rootViewController: picker)
        nav.modalPresentationStyle = .pageSheet
        if let sheet = nav.sheetPresentationController {
            sheet.detents = [
                .custom(identifier: .init("fit")) { [weak picker] ctx in
                    picker?.fittedHeight(ctx.maximumDetentValue) ?? ctx.maximumDetentValue * 0.55
                },
                .large(),
            ]
            sheet.prefersGrabberVisible = true
            sheet.prefersScrollingExpandsWhenScrolledToEdge = true
            picker.sheet = sheet
        }
        return nav
    }

    private init(tabs: [ModelPickerTab], harness: String, model: String?, host: (name: String, symbol: String), note: String?, onPick: @escaping (String, ModelPickerRow) -> Void) {
        self.tabs = tabs
        self.host = host
        self.note = note
        self.onPick = onPick
        self.selected = tabs.firstIndex { $0.harness == harness } ?? 0
        super.init(nibName: nil, bundle: nil)
        pages = tabs.enumerated().map { ix, tab in
            Page(tab: tab, current: (harness, model), footer: tab.footer ?? (ix == selected ? note : nil))
        }
    }

    required init?(coder: NSCoder) { fatalError() }

    override func viewDidLoad() {
        super.viewDidLoad()
        // Clear: the sheet's own glass is the surface.
        view.backgroundColor = .clear
        navigationItem.titleView = titleView()
        navigationItem.leftBarButtonItem = UIBarButtonItem(systemItem: .close, primaryAction: UIAction { [weak self] _ in
            self?.dismiss(animated: true)
        })
        let bar = UINavigationBarAppearance()
        bar.configureWithTransparentBackground()
        navigationItem.standardAppearance = bar
        navigationItem.scrollEdgeAppearance = bar

        pager.isPagingEnabled = true
        pager.showsHorizontalScrollIndicator = false
        pager.alwaysBounceHorizontal = tabs.count > 1
        pager.contentInsetAdjustmentBehavior = .never
        pager.delegate = self
        pager.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(pager)
        for page in pages {
            page.onPick = { [weak self] harness, row in self?.pick(harness, row) }
            page.onLoaded = { [weak self] in self?.refit() }
            pager.addSubview(page)
        }

        let top: NSLayoutYAxisAnchor
        if tabs.count > 1 {
            let switcher = ProviderSwitcher(items: tabs.map { ($0.harness, $0.label) })
            switcher.onSelect = { [weak self] ix in self?.select(ix, animated: true) }
            switcher.translatesAutoresizingMaskIntoConstraints = false
            view.addSubview(switcher)
            NSLayoutConstraint.activate([
                switcher.topAnchor.constraint(equalTo: view.safeAreaLayoutGuide.topAnchor, constant: 6),
                switcher.leadingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.leadingAnchor, constant: 20),
                switcher.trailingAnchor.constraint(equalTo: view.safeAreaLayoutGuide.trailingAnchor, constant: -20),
                switcher.heightAnchor.constraint(equalToConstant: ProviderSwitcher.height),
            ])
            self.switcher = switcher
            top = switcher.bottomAnchor
        } else {
            top = view.safeAreaLayoutGuide.topAnchor
        }
        NSLayoutConstraint.activate([
            pager.topAnchor.constraint(equalTo: top, constant: 4),
            pager.leadingAnchor.constraint(equalTo: view.leadingAnchor),
            pager.trailingAnchor.constraint(equalTo: view.trailingAnchor),
            pager.bottomAnchor.constraint(equalTo: view.bottomAnchor),
        ])
        switcher?.setPosition(CGFloat(selected))
        // The current provider first, then the rest, so a swipe lands on
        // a loaded list.
        pages[selected].load()
        for (ix, page) in pages.enumerated() where ix != selected { page.load() }
        setContentScrollView(pages[selected].list, for: .top)
    }

    override func viewDidLayoutSubviews() {
        super.viewDidLayoutSubviews()
        let size = pager.bounds.size
        guard size.width > 0 else { return }
        for (ix, page) in pages.enumerated() {
            page.frame = CGRect(x: CGFloat(ix) * size.width, y: 0, width: size.width, height: size.height)
        }
        pager.contentSize = CGSize(width: size.width * CGFloat(pages.count), height: size.height)
        if !pager.isDragging && !pager.isDecelerating {
            pager.contentOffset.x = CGFloat(selected) * size.width
        }
    }

    // MARK: Title

    /// "Model" over where it runs; with one provider, that provider too.
    private func titleView() -> UIView {
        let title = UILabel()
        title.text = "Model"
        title.font = Fonts.ui(.sansSemibold, 16)
        title.textColor = Palette.text
        let sub = UILabel()
        sub.font = Fonts.ui(.sans, 12)
        sub.textColor = Palette.secondary
        let icon = UIImageView()
        icon.contentMode = .scaleAspectFit
        icon.tintColor = Palette.secondary
        if tabs.count == 1, let only = tabs.first {
            sub.text = "\(only.label) · \(host.name)"
            icon.image = BrandMarks.image(for: only.harness, side: 12)
        } else {
            sub.text = host.name
            icon.image = UIImage(systemName: host.symbol, withConfiguration: UIImage.SymbolConfiguration(pointSize: 10, weight: .medium))
        }
        let line = UIStackView(arrangedSubviews: [icon, sub])
        line.spacing = 4
        line.alignment = .center
        let stack = UIStackView(arrangedSubviews: [title, line])
        stack.axis = .vertical
        stack.alignment = .center
        stack.spacing = 1
        stack.accessibilityIdentifier = "model-picker-title"
        return stack
    }

    // MARK: Paging

    private func select(_ ix: Int, animated: Bool) {
        guard ix != selected, pages.indices.contains(ix) else { return }
        settle(on: ix)
        let x = CGFloat(ix) * pager.bounds.width
        let move = {
            self.pager.contentOffset.x = x
            self.switcher?.setPosition(CGFloat(ix))
        }
        guard animated, !UIAccessibility.isReduceMotionEnabled else { return move() }
        UIView.animate(withDuration: 0.45, delay: 0, usingSpringWithDamping: 0.86, initialSpringVelocity: 0, options: [.allowUserInteraction, .beginFromCurrentState], animations: move)
    }

    private func settle(on ix: Int) {
        guard ix != selected else { return }
        selected = ix
        UISelectionFeedbackGenerator().selectionChanged()
        view.endEditing(true)
        setContentScrollView(pages[ix].list, for: .top)
    }

    func scrollViewDidScroll(_ scrollView: UIScrollView) {
        guard scrollView === pager, pager.bounds.width > 0, pager.isTracking || pager.isDecelerating else { return }
        let position = min(max(pager.contentOffset.x / pager.bounds.width, 0), CGFloat(pages.count - 1))
        switcher?.setPosition(position)
        settle(on: Int(position.rounded()))
    }

    // MARK: Sizing

    /// Tall enough for the longest provider's list, up to most of the screen.
    private func fittedHeight(_ maximum: CGFloat) -> CGFloat {
        let estimate = maximum * 0.55
        guard isViewLoaded, view.window != nil, pager.bounds.width > 0 else { return estimate }
        let lists = pages.map(\.contentHeight).max() ?? 0
        guard lists > 0 else { return estimate }
        return min(maximum * 0.72, max(pager.frame.minY + lists + 8, 280))
    }

    private func refit() {
        guard let sheet, view.window != nil, sheet.selectedDetentIdentifier != .large else { return }
        sheet.animateChanges { sheet.invalidateDetents() }
    }

    override func viewDidAppear(_ animated: Bool) {
        super.viewDidAppear(animated)
        refit()
    }

    // MARK: Picking

    private func pick(_ harness: String, _ row: ModelPickerRow) {
        guard !picked else { return }
        picked = true
        for page in pages { page.setCurrent((harness, row.id)) }
        UIImpactFeedbackGenerator(style: .light).impactOccurred()
        onPick(harness, row)
        // A beat to see the check land.
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.18) { [weak self] in
            self?.dismiss(animated: true)
        }
    }
}

// MARK: - A provider's page

/// One provider's models as an inset list; a search field once the list is
/// long (OpenCode can offer hundreds).
private final class Page: UIView, UICollectionViewDelegate, UISearchTextFieldDelegate {
    private enum Item: Hashable {
        case model(String)
        case placeholder(Int)
    }

    let tab: ModelPickerTab
    var onPick: ((String, ModelPickerRow) -> Void)?
    var onLoaded: (() -> Void)?
    let list: UICollectionView
    private var current: (harness: String, model: String?)
    private let footer: String?
    private var rows: [ModelPickerRow]?
    private var byId: [String: ModelPickerRow] = [:]
    private var query = ""
    private var search: UISearchTextField?
    private var data: UICollectionViewDiffableDataSource<Int, Item>!
    private var started = false
    private var listTop: NSLayoutConstraint!

    static let searchThreshold = 12

    init(tab: ModelPickerTab, current: (harness: String, model: String?), footer: String?) {
        self.tab = tab
        self.current = current
        self.footer = footer
        var config = UICollectionLayoutListConfiguration(appearance: .insetGrouped)
        config.backgroundColor = .clear
        config.footerMode = footer == nil ? .none : .supplementary
        config.separatorConfiguration.color = Palette.hairline
        let layout = UICollectionViewCompositionalLayout { _, env in
            let section = NSCollectionLayoutSection.list(using: config, layoutEnvironment: env)
            section.contentInsets.top = 10
            section.contentInsets.bottom = 12
            return section
        }
        list = UICollectionView(frame: .zero, collectionViewLayout: layout)
        super.init(frame: .zero)
        list.backgroundColor = .clear
        list.keyboardDismissMode = .onDrag
        list.delegate = self
        list.accessibilityIdentifier = "model-picker-list-\(tab.harness)"
        list.translatesAutoresizingMaskIntoConstraints = false
        addSubview(list)
        listTop = list.topAnchor.constraint(equalTo: topAnchor)
        NSLayoutConstraint.activate([
            listTop,
            list.leadingAnchor.constraint(equalTo: leadingAnchor),
            list.trailingAnchor.constraint(equalTo: trailingAnchor),
            list.bottomAnchor.constraint(equalTo: bottomAnchor),
        ])
        configureData()
        apply(animated: false)
    }

    required init?(coder: NSCoder) { fatalError() }

    private func configureData() {
        let modelCell = UICollectionView.CellRegistration<UICollectionViewListCell, String> { [weak self] cell, _, id in
            guard let self, let row = self.byId[id] else { return }
            var content = UIListContentConfiguration.subtitleCell()
            content.text = row.label
            content.textProperties.font = Fonts.ui(.sansMedium, 16)
            content.textProperties.color = Palette.text
            content.secondaryText = row.detail
            content.secondaryTextProperties.font = Fonts.ui(.sans, 13)
            content.secondaryTextProperties.color = Palette.secondary
            content.secondaryTextProperties.numberOfLines = 2
            content.textToSecondaryTextVerticalPadding = 3
            content.directionalLayoutMargins = NSDirectionalEdgeInsets(top: 11, leading: 18, bottom: 11, trailing: 14)
            cell.contentConfiguration = content
            let isCurrent = self.tab.harness == self.current.harness && id == self.current.model
            cell.accessories = isCurrent
                ? [.checkmark(displayed: .always, options: .init(tintColor: Palette.accent))]
                : []
            cell.configurationUpdateHandler = { cell, state in
                var bg = UIBackgroundConfiguration.listCell().updated(for: state)
                bg.backgroundColor = state.isHighlighted || state.isSelected ? Page.pressed : Palette.rowActive
                cell.backgroundConfiguration = bg
            }
            cell.accessibilityIdentifier = "model-picker-row-\(id)"
            cell.accessibilityTraits = isCurrent ? [.button, .selected] : .button
        }
        let placeholderCell = UICollectionView.CellRegistration<SkeletonCell, Int> { cell, _, ix in
            cell.configure(seed: ix)
        }
        data = UICollectionViewDiffableDataSource(collectionView: list) { list, indexPath, item in
            switch item {
            case .model(let id): list.dequeueConfiguredReusableCell(using: modelCell, for: indexPath, item: id)
            case .placeholder(let ix): list.dequeueConfiguredReusableCell(using: placeholderCell, for: indexPath, item: ix)
            }
        }
        let footerView = UICollectionView.SupplementaryRegistration<UICollectionViewListCell>(elementKind: UICollectionView.elementKindSectionFooter) { [weak self] view, _, _ in
            var content = UIListContentConfiguration.footer()
            content.text = self?.footer
            content.textProperties.font = Fonts.ui(.sans, 13)
            content.textProperties.color = Palette.secondary
            view.contentConfiguration = content
        }
        data.supplementaryViewProvider = { list, _, indexPath in
            list.dequeueConfiguredReusableSupplementary(using: footerView, for: indexPath)
        }
    }

    private static let pressed = Palette.dual(UIColor(white: 0.84, alpha: 0.9), UIColor(white: 1, alpha: 0.13))

    func load() {
        guard !started else { return }
        started = true
        Task { @MainActor [weak self] in
            guard let self else { return }
            var seen = Set<String>()
            let rows = await self.tab.load().filter { seen.insert($0.id).inserted }
            self.rows = rows
            self.byId = Dictionary(uniqueKeysWithValues: rows.map { ($0.id, $0) })
            if rows.count > Self.searchThreshold { self.addSearch() }
            self.apply(animated: true)
            self.onLoaded?()
        }
    }

    func setCurrent(_ current: (harness: String, model: String?)) {
        self.current = current
        var snap = data.snapshot()
        snap.reconfigureItems(snap.itemIdentifiers)
        data.apply(snap, animatingDifferences: false)
    }

    /// The list's natural height (search included), for the sheet's fit.
    var contentHeight: CGFloat {
        list.layoutIfNeeded()
        return list.collectionViewLayout.collectionViewContentSize.height + listTop.constant
    }

    private var visible: [ModelPickerRow] {
        let all = rows ?? []
        let q = query.trimmingCharacters(in: .whitespaces).lowercased()
        guard !q.isEmpty else { return all }
        return all.filter { $0.label.lowercased().contains(q) || $0.id.lowercased().contains(q) }
    }

    private func apply(animated: Bool) {
        var snap = NSDiffableDataSourceSnapshot<Int, Item>()
        snap.appendSections([0])
        if rows == nil {
            snap.appendItems((0..<3).map(Item.placeholder))
        } else {
            snap.appendItems(visible.map { .model($0.id) })
        }
        data.apply(snap, animatingDifferences: animated && !UIAccessibility.isReduceMotionEnabled)
        list.backgroundView = rows != nil && visible.isEmpty ? emptyLabel() : nil
    }

    private func emptyLabel() -> UIView {
        let label = UILabel()
        label.text = query.isEmpty ? "No models available" : "No matches"
        label.font = Fonts.ui(.sans, 15)
        label.textColor = Palette.tertiary
        label.textAlignment = .center
        let holder = UIView()
        holder.addSubview(label)
        label.translatesAutoresizingMaskIntoConstraints = false
        NSLayoutConstraint.activate([
            label.centerXAnchor.constraint(equalTo: holder.centerXAnchor),
            label.topAnchor.constraint(equalTo: holder.topAnchor, constant: 40),
        ])
        return holder
    }

    private func addSearch() {
        let field = UISearchTextField()
        field.placeholder = "Search \(tab.label) models"
        field.font = Fonts.ui(.sans, 15)
        field.returnKeyType = .done
        field.delegate = self
        field.accessibilityIdentifier = "model-picker-search"
        field.addAction(UIAction { [weak self, weak field] _ in
            self?.query = field?.text ?? ""
            self?.apply(animated: false)
        }, for: .editingChanged)
        field.translatesAutoresizingMaskIntoConstraints = false
        addSubview(field)
        NSLayoutConstraint.activate([
            field.topAnchor.constraint(equalTo: topAnchor, constant: 8),
            field.leadingAnchor.constraint(equalTo: safeAreaLayoutGuide.leadingAnchor, constant: 20),
            field.trailingAnchor.constraint(equalTo: safeAreaLayoutGuide.trailingAnchor, constant: -20),
        ])
        listTop.constant = 52
        search = field
    }

    func textFieldShouldReturn(_ textField: UITextField) -> Bool {
        textField.resignFirstResponder()
        return true
    }

    func collectionView(_ collectionView: UICollectionView, didSelectItemAt indexPath: IndexPath) {
        collectionView.deselectItem(at: indexPath, animated: true)
        guard case .model(let id) = data.itemIdentifier(for: indexPath), let row = byId[id] else { return }
        onPick?(tab.harness, row)
    }

    func collectionView(_ collectionView: UICollectionView, shouldHighlightItemAt indexPath: IndexPath) -> Bool {
        if case .model = data.itemIdentifier(for: indexPath) { return true }
        return false
    }
}

/// A loading row: two soft bars that breathe until the models arrive.
private final class SkeletonCell: UICollectionViewListCell {
    private let title = UIView()
    private let detail = UIView()
    private var widths: [NSLayoutConstraint] = []

    override init(frame: CGRect) {
        super.init(frame: frame)
        for bar in [title, detail] {
            bar.backgroundColor = Palette.controlFill
            bar.layer.cornerRadius = 4
            bar.layer.cornerCurve = .continuous
            bar.translatesAutoresizingMaskIntoConstraints = false
            contentView.addSubview(bar)
        }
        NSLayoutConstraint.activate([
            title.topAnchor.constraint(equalTo: contentView.topAnchor, constant: 17),
            title.leadingAnchor.constraint(equalTo: contentView.leadingAnchor, constant: 18),
            title.heightAnchor.constraint(equalToConstant: 12),
            detail.topAnchor.constraint(equalTo: title.bottomAnchor, constant: 9),
            detail.leadingAnchor.constraint(equalTo: title.leadingAnchor),
            detail.heightAnchor.constraint(equalToConstant: 9),
            detail.bottomAnchor.constraint(equalTo: contentView.bottomAnchor, constant: -17),
        ])
        var bg = UIBackgroundConfiguration.listCell()
        bg.backgroundColor = Palette.rowActive
        backgroundConfiguration = bg
        isUserInteractionEnabled = false
    }

    required init?(coder: NSCoder) { fatalError() }

    func configure(seed: Int) {
        NSLayoutConstraint.deactivate(widths)
        let t: [CGFloat] = [0.34, 0.42, 0.28]
        let d: [CGFloat] = [0.62, 0.5, 0.56]
        widths = [
            title.widthAnchor.constraint(equalTo: contentView.widthAnchor, multiplier: t[seed % 3]),
            detail.widthAnchor.constraint(equalTo: contentView.widthAnchor, multiplier: d[seed % 3]),
        ]
        NSLayoutConstraint.activate(widths)
        guard !UIAccessibility.isReduceMotionEnabled else { return }
        for bar in [title, detail] {
            bar.layer.removeAllAnimations()
            let pulse = CABasicAnimation(keyPath: "opacity")
            pulse.fromValue = 1
            pulse.toValue = 0.4
            pulse.duration = 0.9
            pulse.beginTime = CACurrentMediaTime() + Double(seed) * 0.12
            pulse.autoreverses = true
            pulse.repeatCount = .infinity
            pulse.timingFunction = CAMediaTimingFunction(name: .easeInEaseOut)
            bar.layer.add(pulse, forKey: "pulse")
        }
    }
}

// MARK: - Provider switcher

/// A capsule track of provider segments (logo + name) with a thumb that
/// follows the pages under it, finger for finger. When the names don't all
/// fit, the others shrink to their logos and the chosen one keeps its name.
private final class ProviderSwitcher: UIView {
    static let height: CGFloat = 40
    private static let inset: CGFloat = 3

    var onSelect: ((Int) -> Void)?
    private let thumb = UIView()
    private var segments: [Segment] = []
    private var position: CGFloat = 0

    init(items: [(harness: String, title: String)]) {
        super.init(frame: .zero)
        backgroundColor = Palette.controlFill
        layer.cornerRadius = Self.height / 2
        layer.cornerCurve = .continuous
        thumb.backgroundColor = Palette.dual(.white, UIColor(white: 1, alpha: 0.14))
        thumb.layer.cornerCurve = .continuous
        thumb.layer.shadowColor = UIColor.black.cgColor
        thumb.layer.shadowOffset = CGSize(width: 0, height: 1)
        thumb.layer.shadowRadius = 5
        addSubview(thumb)
        for (ix, item) in items.enumerated() {
            // The tab is short on room: "Claude", not "Claude Code".
            let segment = Segment(harness: item.harness, title: item.title == "Claude Code" ? "Claude" : item.title)
            segment.addAction(UIAction { [weak self] _ in self?.onSelect?(ix) }, for: .touchUpInside)
            segments.append(segment)
            addSubview(segment)
        }
        updateShadow()
        registerForTraitChanges([UITraitUserInterfaceStyle.self]) { (self: ProviderSwitcher, _) in self.updateShadow() }
    }

    required init?(coder: NSCoder) { fatalError() }

    private func updateShadow() {
        thumb.layer.shadowOpacity = traitCollection.userInterfaceStyle == .dark ? 0 : 0.1
    }

    /// Fractional: 1.4 = 40% of the way from the second provider to the third.
    func setPosition(_ position: CGFloat) {
        self.position = position
        setNeedsLayout()
        layoutIfNeeded()
    }

    override func layoutSubviews() {
        super.layoutSubviews()
        guard !segments.isEmpty, bounds.width > 0 else { return }
        let lower = max(0, min(segments.count - 1, Int(position.rounded(.down))))
        let upper = min(segments.count - 1, lower + 1)
        let t = min(max(position - CGFloat(lower), 0), 1)
        let a = frames(selected: lower), b = frames(selected: upper)
        let compact = isCompact
        for (k, segment) in segments.enumerated() {
            // Bounds + center, not frame: a pressed segment is scaled.
            let f = lerp(a[k], b[k], t)
            segment.bounds = CGRect(origin: .zero, size: f.size)
            segment.center = CGPoint(x: f.midX, y: f.midY)
            segment.nameVisibility = compact ? lerp(k == lower ? 1 : 0, k == upper ? 1 : 0, t) : 1
            segment.emphasis = max(0, 1 - abs(position - CGFloat(k)))
            segment.setNeedsLayout()
            segment.layoutIfNeeded()
        }
        thumb.frame = lerp(a[lower], b[upper], t)
        thumb.layer.cornerRadius = thumb.bounds.height / 2
    }

    private var available: CGFloat { bounds.width - Self.inset * 2 }

    private var isCompact: Bool { segments.map(\.naturalWidth).reduce(0, +) > available }

    private func frames(selected: Int) -> [CGRect] {
        let natural = segments.map(\.naturalWidth)
        let n = CGFloat(segments.count)
        var widths: [CGFloat]
        if (natural.max() ?? 0) * n <= available {
            widths = Array(repeating: available / n, count: segments.count)
        } else if natural.reduce(0, +) <= available {
            let extra = (available - natural.reduce(0, +)) / n
            widths = natural.map { $0 + extra }
        } else {
            widths = segments.indices.map { $0 == selected ? natural[$0] : Segment.logoWidth }
            let extra = max(0, (available - widths.reduce(0, +)) / n)
            widths = widths.map { $0 + extra }
        }
        var x = Self.inset
        return widths.map { w in
            defer { x += w }
            return CGRect(x: x, y: Self.inset, width: w, height: bounds.height - Self.inset * 2)
        }
    }

    private func lerp(_ a: CGFloat, _ b: CGFloat, _ t: CGFloat) -> CGFloat { a + (b - a) * t }

    private func lerp(_ a: CGRect, _ b: CGRect, _ t: CGFloat) -> CGRect {
        CGRect(x: lerp(a.minX, b.minX, t), y: lerp(a.minY, b.minY, t), width: lerp(a.width, b.width, t), height: lerp(a.height, b.height, t))
    }
}

/// One provider in the switcher. Two stacked names (muted, full) cross-fade
/// with `emphasis`, so the color follows the finger too.
private final class Segment: UIControl {
    static let logoWidth: CGFloat = 46
    private static let logo: CGFloat = 17
    private static let gap: CGFloat = 7

    var nameVisibility: CGFloat = 1
    var emphasis: CGFloat = 0
    private let icon = UIImageView()
    private let muted = UILabel()
    private let full = UILabel()
    private let nameWidth: CGFloat

    init(harness: String, title: String) {
        for label in [muted, full] {
            label.text = title
            label.font = Fonts.ui(.sansMedium, 14)
        }
        muted.textColor = Palette.secondary
        full.textColor = Palette.text
        nameWidth = ceil(full.intrinsicContentSize.width)
        super.init(frame: .zero)
        icon.image = BrandMarks.image(for: harness, side: Self.logo)
        icon.tintColor = Palette.text
        icon.contentMode = .scaleAspectFit
        for v in [icon, muted, full] as [UIView] {
            v.isUserInteractionEnabled = false
            addSubview(v)
        }
        clipsToBounds = true
        isAccessibilityElement = true
        accessibilityLabel = title
        accessibilityIdentifier = "model-picker-tab-\(harness)"
    }

    required init?(coder: NSCoder) { fatalError() }

    var naturalWidth: CGFloat { 14 + Self.logo + Self.gap + nameWidth + 16 }

    override var accessibilityTraits: UIAccessibilityTraits {
        get { emphasis > 0.5 ? [.button, .selected] : .button }
        set {}
    }

    override var isHighlighted: Bool {
        didSet {
            UIView.animate(withDuration: isHighlighted ? 0.08 : 0.25) {
                self.icon.alpha = self.isHighlighted ? 0.45 : self.iconAlpha
                self.transform = self.isHighlighted ? CGAffineTransform(scaleX: 0.96, y: 0.96) : .identity
            }
        }
    }

    private var iconAlpha: CGFloat { 0.55 + 0.45 * emphasis }

    override func layoutSubviews() {
        super.layoutSubviews()
        // Frames in bounds space: the press scale is a transform.
        let size = bounds.size
        let content = Self.logo + (Self.gap + nameWidth) * nameVisibility
        let x0 = ((size.width - content) / 2).rounded()
        icon.frame = CGRect(x: x0, y: ((size.height - Self.logo) / 2).rounded(), width: Self.logo, height: Self.logo)
        icon.alpha = isHighlighted ? 0.45 : iconAlpha
        let h = ceil(full.font.lineHeight)
        let name = CGRect(x: x0 + Self.logo + Self.gap, y: ((size.height - h) / 2).rounded(), width: nameWidth, height: h)
        muted.frame = name
        full.frame = name
        muted.alpha = nameVisibility * (1 - emphasis)
        full.alpha = nameVisibility * emphasis
    }
}
