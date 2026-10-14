package main

import (
	"errors"
	"net/http"
	"time"

	"github.com/jackc/pgx/v5"
)

type commentJSON struct {
	ID        int64       `json:"id"`
	CreatedAt ts          `json:"createdAt"`
	UpdatedAt ts          `json:"updatedAt"`
	Body      string      `json:"body"`
	Author    profileJSON `json:"author"`
}

// GET /api/articles/{slug}/comments
func (a *app) listComments(w http.ResponseWriter, r *http.Request, viewer int64, _ string) {
	id, ok := a.articleID(w, r)
	if !ok {
		return
	}
	rows, _ := a.pool.Query(r.Context(), `SELECT c.id, c.body, c.created_at, c.updated_at, u.username, u.bio, u.image,
       EXISTS (SELECT 1 FROM follows fo WHERE fo.follower_id = $1 AND fo.followee_id = c.author_id) AS following
  FROM comments c JOIN users u ON u.id = c.author_id
 WHERE c.article_id = $2
 ORDER BY c.created_at DESC, c.id DESC LIMIT 100`, viewer, id)
	comments, err := pgx.CollectRows(rows, func(row pgx.CollectableRow) (commentJSON, error) {
		var c commentJSON
		var created, updated time.Time
		err := row.Scan(&c.ID, &c.Body, &created, &updated, &c.Author.Username, &c.Author.Bio, &c.Author.Image, &c.Author.Following)
		c.CreatedAt, c.UpdatedAt = ts(created), ts(updated)
		return c, err
	})
	if err != nil {
		internalError(w, err)
		return
	}
	if comments == nil {
		comments = []commentJSON{}
	}
	writeJSON(w, http.StatusOK, struct {
		Comments []commentJSON `json:"comments"`
	}{comments})
}

// POST /api/articles/{slug}/comments
func (a *app) createComment(w http.ResponseWriter, r *http.Request, viewer int64, _ string) {
	m, err := readWrapper(r, "comment")
	if err != nil {
		writeError(w, http.StatusUnprocessableEntity, "invalid body")
		return
	}
	body, err := reqString(m, "body")
	if err != nil {
		writeError(w, http.StatusUnprocessableEntity, "body is required")
		return
	}
	id, ok := a.articleID(w, r)
	if !ok {
		return
	}
	var c commentJSON
	var created, updated time.Time
	err = a.pool.QueryRow(r.Context(), `WITH c AS (INSERT INTO comments (article_id, author_id, body) VALUES ($1, $2, $3)
           RETURNING id, body, created_at, updated_at, author_id)
SELECT c.id, c.body, c.created_at, c.updated_at, u.username, u.bio, u.image
  FROM c JOIN users u ON u.id = c.author_id`, id, viewer, body).Scan(&c.ID, &c.Body, &created, &updated, &c.Author.Username, &c.Author.Bio, &c.Author.Image)
	if err != nil {
		internalError(w, err)
		return
	}
	c.CreatedAt, c.UpdatedAt = ts(created), ts(updated)
	writeJSON(w, http.StatusCreated, struct {
		Comment commentJSON `json:"comment"`
	}{c})
}

// DELETE /api/articles/{slug}/comments/{id}
func (a *app) deleteComment(w http.ResponseWriter, r *http.Request, viewer int64, _ string) {
	id, ok := parseUint(r.PathValue("id"))
	if !ok {
		writeError(w, http.StatusNotFound, "comment not found")
		return
	}
	var authorID int64
	err := a.pool.QueryRow(r.Context(), `SELECT c.author_id FROM comments c JOIN articles a ON a.id = c.article_id
 WHERE c.id = $1 AND a.slug = $2`, id, r.PathValue("slug")).Scan(&authorID)
	if errors.Is(err, pgx.ErrNoRows) {
		writeError(w, http.StatusNotFound, "comment not found")
		return
	}
	if err != nil {
		internalError(w, err)
		return
	}
	if authorID != viewer {
		writeError(w, http.StatusForbidden, "not the author")
		return
	}
	if _, err := a.pool.Exec(r.Context(), `DELETE FROM comments WHERE id = $1`, id); err != nil {
		internalError(w, err)
		return
	}
	w.WriteHeader(http.StatusNoContent)
}
