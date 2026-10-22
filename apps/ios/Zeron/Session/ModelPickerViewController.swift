import UIKit

/// One provider in the model picker: its logo tab and its models, loaded
/// when the tab is first shown.
struct ModelPickerTab {
    let harness: String
    let label: String
    let load: () async -> [ModelPickerRow]
}

struct ModelPickerRow: Equatable {
    let id: String
    let label: String
    let detail: String?
}

/// The model picker as a sheet: a tab per provider (the current one first),
/// a search, and the selected provider's models. Other providers' tabs are
/// only there when a switch is allowed; `note` says why when it isn't.
final class ModelPickerViewController: UIViewController, UITableViewDataSource, UITableViewDelegate, UISearchTextFieldDelegate {
    private let tabs: [ModelPickerTab]
    private let current: (harness: String, model: String?)
    private let note: String?
    private let onPick: (_ harness: String, _ model: ModelPickerRow) -> Void

    private var selected: Int
    private var loaded: [String: [ModelPickerRow]] = [:]
    private var query = ""
    private let tabStrip = UIStackView()
    private let search = UISearchTextField()
    private let table = UITableView(frame: .zero, style: .plain)
    private let spinner = UIActivityIndicatorView(style: .medium)

    init(tabs: [ModelPickerTab], harness: String, model: String?, note: String? = nil, onPick: @escaping (String, ModelPickerRow) -> Void) {
        self.tabs = tabs
        self.current = (harness, model)
        self.note = note
        self.onPick = onPick
        self.selected = tabs.firstIndex { $0.harness == harness } ?? 0
        super.init(nibName: nil, bundle: nil)
        modalPresentationStyle = .pageSheet
        if let sheet = sheetPresentationController {
            sheet.detents = [.medium(), .large()]
            sheet.prefersGrabberVisible = true
            sheet.prefersScrollingExpandsWhenScrolledToEdge = true
        }
    }

    required init?(coder: NSCoder) { fatalError() }

    override func viewDidLoad() {
        super.viewDidLoad()
        view.backgroundColor = Palette.background

        let title = UILabel()
        title.text = "Model"
        title.font = Fonts.ui(.sansSemibold, 17)
        title.textColor = Palette.text

        tabStrip.axis = .horizontal
        tabStrip.spacing = 6
        let tabScroll = UIScrollView()
        tabScroll.showsHorizontalScrollIndicator = false
        tabScroll.addSubview(tabStrip)
        tabStrip.translatesAutoresizingMaskIntoConstraints = false
        rebuildTabs()

        search.placeholder = "Search models"
        search.font = Fonts.ui(.sans, 15)
        search.delegate = self
        search.returnKeyType = .done
        search.addAction(UIAction { [weak self] _ in
            guard let self else { return }
            self.query = self.search.text ?? ""
            self.table.reloadData()
        }, for: .editingChanged)
        search.accessibilityIdentifier = "model-picker-search"

        let header = UIStackView(arrangedSubviews: [title, tabScroll, search])
        header.axis = .vertical
        header.spacing = 12
        if let note {
            let label = UILabel()
            label.text = note
            label.font = Fonts.ui(.sans, 13)
            label.textColor = Palette.secondary
            label.numberOfLines = 0
            header.insertArrangedSubview(label, at: 2)
        }
        header.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(header)

        table.dataSource = self
        table.delegate = self
        table.backgroundColor = .clear
        table.separatorStyle = .none
        table.keyboardDismissMode = .onDrag
        table.register(UITableViewCell.self, forCellReuseIdentifier: "model")
        table.translatesAutoresizingMaskIntoConstraints = false
        table.accessibilityIdentifier = "model-picker-list"
        view.addSubview(table)
        spinner.hidesWhenStopped = true
        spinner.translatesAutoresizingMaskIntoConstraints = false
        view.addSubview(spinner)

        NSLayoutConstraint.activate([
            header.topAnchor.constraint(equalTo: view.safeAreaLayoutGuide.topAnchor, constant: 22),
            header.leadingAnchor.constraint(equalTo: view.leadingAnchor, constant: 20),
            header.trailingAnchor.constraint(equalTo: view.trailingAnchor, constant: -20),
            tabScroll.heightAnchor.constraint(equalToConstant: 36),
            tabStrip.topAnchor.constraint(equalTo: tabScroll.contentLayoutGuide.topAnchor),
            tabStrip.bottomAnchor.constraint(equalTo: tabScroll.contentLayoutGuide.bottomAnchor),
            tabStrip.leadingAnchor.constraint(equalTo: tabScroll.contentLayoutGuide.leadingAnchor),
            tabStrip.trailingAnchor.constraint(equalTo: tabScroll.contentLayoutGuide.trailingAnchor),
            tabStrip.heightAnchor.constraint(equalTo: tabScroll.frameLayoutGuide.heightAnchor),
            table.topAnchor.constraint(equalTo: header.bottomAnchor, constant: 8),
            table.leadingAnchor.constraint(equalTo: view.leadingAnchor),
            table.trailingAnchor.constraint(equalTo: view.trailingAnchor),
            table.bottomAnchor.constraint(equalTo: view.bottomAnchor),
            spinner.centerXAnchor.constraint(equalTo: table.centerXAnchor),
            spinner.topAnchor.constraint(equalTo: table.topAnchor, constant: 32),
        ])
        loadSelected()
    }

    // MARK: Tabs

    private func rebuildTabs() {
        tabStrip.arrangedSubviews.forEach { $0.removeFromSuperview() }
        for (ix, tab) in tabs.enumerated() {
            var c = UIButton.Configuration.plain()
            c.image = BrandMarks.image(for: tab.harness, side: 16)
            c.imagePadding = 6
            c.title = tab.label
            c.cornerStyle = .capsule
            c.contentInsets = NSDirectionalEdgeInsets(top: 7, leading: 12, bottom: 7, trailing: 14)
            c.baseForegroundColor = ix == selected ? Palette.text : Palette.secondary
            c.background.backgroundColor = ix == selected ? Palette.controlFill : .clear
            c.titleTextAttributesTransformer = UIConfigurationTextAttributesTransformer { a in
                var a = a
                a.font = Fonts.ui(.sansMedium, 14)
                return a
            }
            let b = UIButton(configuration: c, primaryAction: UIAction { [weak self] _ in self?.select(ix) })
            b.accessibilityIdentifier = "model-picker-tab-\(tab.harness)"
            b.accessibilityTraits.insert(ix == selected ? .selected : [])
            tabStrip.addArrangedSubview(b)
        }
    }

    private func select(_ ix: Int) {
        guard ix != selected else { return }
        UISelectionFeedbackGenerator().selectionChanged()
        selected = ix
        rebuildTabs()
        table.reloadData()
        loadSelected()
    }

    private func loadSelected() {
        let tab = tabs[selected]
        guard loaded[tab.harness] == nil else { return spinner.stopAnimating() }
        spinner.startAnimating()
        Task { @MainActor [weak self] in
            let rows = await tab.load()
            guard let self else { return }
            self.loaded[tab.harness] = rows
            if self.tabs[self.selected].harness == tab.harness {
                self.spinner.stopAnimating()
                self.table.reloadData()
            }
        }
    }

    // MARK: List

    private var rows: [ModelPickerRow] {
        let all = loaded[tabs[selected].harness] ?? []
        let q = query.trimmingCharacters(in: .whitespaces).lowercased()
        guard !q.isEmpty else { return all }
        return all.filter { $0.label.lowercased().contains(q) || $0.id.lowercased().contains(q) }
    }

    func tableView(_ tableView: UITableView, numberOfRowsInSection section: Int) -> Int { rows.count }

    func tableView(_ tableView: UITableView, cellForRowAt indexPath: IndexPath) -> UITableViewCell {
        let row = rows[indexPath.row]
        let harness = tabs[selected].harness
        let cell = tableView.dequeueReusableCell(withIdentifier: "model", for: indexPath)
        var content = UIListContentConfiguration.subtitleCell()
        content.text = row.label
        content.textProperties.font = Fonts.ui(.sansMedium, 16)
        content.textProperties.color = Palette.text
        content.secondaryText = row.detail
        content.secondaryTextProperties.font = Fonts.ui(.sans, 13)
        content.secondaryTextProperties.color = Palette.secondary
        content.image = BrandMarks.image(for: harness, side: 18)
        content.imageProperties.reservedLayoutSize = CGSize(width: 22, height: 22)
        content.imageToTextPadding = 12
        content.directionalLayoutMargins = NSDirectionalEdgeInsets(top: 11, leading: 20, bottom: 11, trailing: 20)
        cell.contentConfiguration = content
        cell.backgroundColor = .clear
        let isCurrent = harness == current.harness && row.id == current.model
        cell.accessoryType = isCurrent ? .checkmark : .none
        cell.tintColor = Palette.accent
        cell.accessibilityIdentifier = "model-picker-row-\(row.id)"
        return cell
    }

    func tableView(_ tableView: UITableView, didSelectRowAt indexPath: IndexPath) {
        tableView.deselectRow(at: indexPath, animated: true)
        let row = rows[indexPath.row]
        let harness = tabs[selected].harness
        UISelectionFeedbackGenerator().selectionChanged()
        onPick(harness, row)
        dismiss(animated: true)
    }

    func textFieldShouldReturn(_ textField: UITextField) -> Bool {
        textField.resignFirstResponder()
        return true
    }
}
