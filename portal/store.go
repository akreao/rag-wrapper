package main

import (
	"context"
	"database/sql"
	"errors"
	"strings"
	"sync"
	"time"
)

// blockedState is the login.state value rAthena's @block command sets: the
// login server refuses the account until state is back to 0.
const blockedState = 5

// Account is a row of rAthena's login table. Server accounts (sex 'S', such
// as the inter-server login) are never loaded.
type Account struct {
	ID         int
	UserID     string
	Sex        string
	Email      string
	GroupID    int
	State      int64
	UnbanTime  int64
	LoginCount int
	LastLogin  string
	LastIP     string
	Birthdate  string // YYYY-MM-DD, or "" when unset
	passHash   string
}

// Blocked reports whether the login server would refuse this account.
func (a *Account) Blocked() bool { return a.State != 0 }

// Banned reports whether a temporary ban is still running.
func (a *Account) Banned() bool { return a.UnbanTime > time.Now().Unix() }

// BannedUntil is the end of the temporary ban.
func (a *Account) BannedUntil() time.Time { return time.Unix(a.UnbanTime, 0) }

// Character is a row of rAthena's char table.
type Character struct {
	Slot      int
	Name      string
	Class     int
	BaseLevel int
	JobLevel  int
	Zeny      int64
	LastMap   string
	Online    bool
	Deleting  bool
}

func (c Character) JobName() string { return jobName(c.Class) }

var errUserIDTaken = errors.New("that username is already taken")

type store struct {
	db *sql.DB
	// The login table has no unique key on userid, so registrations are
	// serialised here to keep two people from claiming the same name.
	registerMu sync.Mutex
}

const accountColumns = "`account_id`, `userid`, `user_pass`, `sex`, `email`, `group_id`, `state`, `unban_time`, `logincount`, " +
	"COALESCE(CAST(`lastlogin` AS CHAR), ''), `last_ip`, COALESCE(CAST(`birthdate` AS CHAR), '')"

func scanAccount(row interface{ Scan(...any) error }) (*Account, error) {
	a := &Account{}
	err := row.Scan(&a.ID, &a.UserID, &a.passHash, &a.Sex, &a.Email, &a.GroupID, &a.State, &a.UnbanTime,
		&a.LoginCount, &a.LastLogin, &a.LastIP, &a.Birthdate)
	if err != nil {
		return nil, err
	}
	if a.Birthdate == "0000-00-00" {
		a.Birthdate = ""
	}
	return a, nil
}

// accountByUserID returns nil, nil when there's no such account.
func (s *store) accountByUserID(ctx context.Context, userID string) (*Account, error) {
	a, err := scanAccount(s.db.QueryRowContext(ctx,
		"SELECT "+accountColumns+" FROM `login` WHERE `userid` = ? AND `sex` <> 'S'", userID))
	if errors.Is(err, sql.ErrNoRows) {
		return nil, nil
	}
	return a, err
}

// accountByID returns nil, nil when there's no such account.
func (s *store) accountByID(ctx context.Context, id int) (*Account, error) {
	a, err := scanAccount(s.db.QueryRowContext(ctx,
		"SELECT "+accountColumns+" FROM `login` WHERE `account_id` = ? AND `sex` <> 'S'", id))
	if errors.Is(err, sql.ErrNoRows) {
		return nil, nil
	}
	return a, err
}

func (s *store) createAccount(ctx context.Context, userID, passHash, sex, email, birthdate string) (int, error) {
	s.registerMu.Lock()
	defer s.registerMu.Unlock()

	// userid uses a case-insensitive collation, as the login server's
	// lookups do, so "Bob" and "bob" count as the same name.
	var exists int
	err := s.db.QueryRowContext(ctx, "SELECT COUNT(*) FROM `login` WHERE `userid` = ?", userID).Scan(&exists)
	if err != nil {
		return 0, err
	}
	if exists > 0 {
		return 0, errUserIDTaken
	}
	res, err := s.db.ExecContext(ctx,
		"INSERT INTO `login` (`userid`, `user_pass`, `sex`, `email`, `birthdate`) VALUES (?, ?, ?, ?, ?)",
		userID, passHash, sex, email, nullIfEmpty(birthdate))
	if err != nil {
		return 0, err
	}
	id, err := res.LastInsertId()
	return int(id), err
}

func (s *store) setPassword(ctx context.Context, id int, passHash string) error {
	_, err := s.db.ExecContext(ctx, "UPDATE `login` SET `user_pass` = ? WHERE `account_id` = ? AND `sex` <> 'S'", passHash, id)
	return err
}

func (s *store) setDetails(ctx context.Context, id int, email, birthdate string) error {
	_, err := s.db.ExecContext(ctx, "UPDATE `login` SET `email` = ?, `birthdate` = ? WHERE `account_id` = ? AND `sex` <> 'S'",
		email, nullIfEmpty(birthdate), id)
	return err
}

func (s *store) characters(ctx context.Context, accountID int) ([]Character, error) {
	rows, err := s.db.QueryContext(ctx,
		"SELECT `char_num`, `name`, `class`, `base_level`, `job_level`, `zeny`, `last_map`, `online`, `delete_date` "+
			"FROM `char` WHERE `account_id` = ? ORDER BY `char_num`", accountID)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	var chars []Character
	for rows.Next() {
		var c Character
		var online, deleteDate int64
		if err := rows.Scan(&c.Slot, &c.Name, &c.Class, &c.BaseLevel, &c.JobLevel, &c.Zeny, &c.LastMap, &online, &deleteDate); err != nil {
			return nil, err
		}
		c.Online = online != 0
		c.Deleting = deleteDate != 0
		chars = append(chars, c)
	}
	return chars, rows.Err()
}

// searchAccounts lists player accounts whose name or email contains q,
// newest first.
func (s *store) searchAccounts(ctx context.Context, q string, limit, offset int) ([]*Account, error) {
	like := "%" + escapeLike(q) + "%"
	rows, err := s.db.QueryContext(ctx,
		"SELECT "+accountColumns+" FROM `login` WHERE `sex` <> 'S' AND (`userid` LIKE ? OR `email` LIKE ?) "+
			"ORDER BY `account_id` DESC LIMIT ? OFFSET ?", like, like, limit, offset)
	if err != nil {
		return nil, err
	}
	defer rows.Close()
	var accounts []*Account
	for rows.Next() {
		a, err := scanAccount(rows)
		if err != nil {
			return nil, err
		}
		accounts = append(accounts, a)
	}
	return accounts, rows.Err()
}

func (s *store) setGroup(ctx context.Context, id, group int) error {
	_, err := s.db.ExecContext(ctx, "UPDATE `login` SET `group_id` = ? WHERE `account_id` = ? AND `sex` <> 'S'", group, id)
	return err
}

// ban refuses logins until the given time, like @ban.
func (s *store) ban(ctx context.Context, id int, until time.Time) error {
	_, err := s.db.ExecContext(ctx, "UPDATE `login` SET `unban_time` = ? WHERE `account_id` = ? AND `sex` <> 'S'", until.Unix(), id)
	return err
}

// block refuses logins until unblocked, like @block.
func (s *store) block(ctx context.Context, id int) error {
	_, err := s.db.ExecContext(ctx, "UPDATE `login` SET `state` = ? WHERE `account_id` = ? AND `sex` <> 'S'", blockedState, id)
	return err
}

// unban lifts both a temporary ban and a block.
func (s *store) unban(ctx context.Context, id int) error {
	_, err := s.db.ExecContext(ctx, "UPDATE `login` SET `state` = 0, `unban_time` = 0 WHERE `account_id` = ? AND `sex` <> 'S'", id)
	return err
}

func (s *store) ping(ctx context.Context) error { return s.db.PingContext(ctx) }

func nullIfEmpty(s string) any {
	if s == "" {
		return nil
	}
	return s
}

func escapeLike(s string) string {
	return strings.NewReplacer(`\`, `\\`, `%`, `\%`, `_`, `\_`).Replace(s)
}
